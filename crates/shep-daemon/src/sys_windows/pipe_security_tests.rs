//! The descriptor [`UserOnly`] puts on a pipe, read back from the pipe itself
//!
//! A second local account cannot be signed in from a test, so the proof is
//! the descriptor the kernel holds for the pipe: allow ACEs for the user this
//! process runs as, and for no one else. A pipe whose DACL names only that SID
//! refuses every other, `Everyone` included. The round trip beside it shows
//! the user is not locked out of their own pipe.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use shep_core::transport::{Listener, connect};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_KERNEL_OBJECT};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, GetAce,
};

use super::*;

/// `ACCESS_ALLOWED_ACE_TYPE`, which `windows-sys` does not export.
const ACCESS_ALLOWED: u8 = 0;

/// Every await bounded, so a hang fails this test and not the whole binary.
const BOUND: Duration = Duration::from_secs(10);

/// A pipe name no other test, and no other run of this one, is using.
fn pipe_name(tag: &str) -> PathBuf {
    PathBuf::from(format!(
        r"\\.\pipe\shep-pipe-security-test-{tag}-{}",
        std::process::id()
    ))
}

/// The `(ace type, SID)` of every entry in `pipe`'s DACL.
fn dacl_of(pipe: &NamedPipeServer) -> Vec<(u8, Vec<u16>)> {
    let mut dacl: *mut ACL = core::ptr::null_mut();
    let mut descriptor: *mut c_void = core::ptr::null_mut();
    // SAFETY: the handle is live for as long as `pipe` is, and a pipe's own
    // handle carries `READ_CONTROL`. Only the DACL is asked for, so the
    // other out pointers are the documented nulls. On success `dacl` points
    // into `descriptor`, which is freed at the end of this function.
    let status = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &raw mut dacl,
            core::ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    assert_eq!(
        status, ERROR_SUCCESS,
        "the pipe's security must be readable"
    );
    assert!(!dacl.is_null(), "a pipe with no DACL admits everyone");

    // SAFETY: `dacl` is non-null and points at the ACL inside `descriptor`.
    let count = unsafe { (*dacl).AceCount };
    let mut entries = Vec::new();
    for index in 0..u32::from(count) {
        let mut ace: *mut c_void = core::ptr::null_mut();
        // SAFETY: `index` is below the ACL's own count, and `ace` is a live
        // local for the out pointer.
        let found = unsafe { GetAce(dacl, index, &raw mut ace) };
        assert_ne!(found, 0, "ace {index} must be readable");
        // SAFETY: `ace` points at an ACE inside `descriptor`, and every ACE
        // starts with an `ACE_HEADER`.
        let kind = unsafe { (*ace.cast::<ACE_HEADER>()).AceType };
        // SAFETY: an allow ACE ends in its SID, which starts at `SidStart`,
        // and the SID is inside `descriptor`, alive until the free below.
        let sid = unsafe {
            sid_to_wide(
                core::ptr::addr_of_mut!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast(),
            )
        }
        .expect("an ACE's SID must have a string form");
        entries.push((kind, sid));
    }
    // SAFETY: `descriptor` was allocated by `GetSecurityInfo`, which
    // documents `LocalFree` as its release, and nothing reads it after.
    unsafe {
        let _ = LocalFree(descriptor);
    }
    entries
}

/// fails if a pipe made under the descriptor admits anyone besides the user the
/// process runs as: a second ACE, a deny, or no DACL at all.
#[tokio::test]
async fn a_pipe_under_the_descriptor_admits_only_the_current_user() {
    let security = UserOnly::current_user().unwrap();
    let pipe = security
        .create(
            ServerOptions::new().first_pipe_instance(true),
            pipe_name("dacl").as_os_str(),
        )
        .unwrap();

    let entries = dacl_of(&pipe);

    assert_eq!(
        entries,
        vec![(ACCESS_ALLOWED, current_user_sid().unwrap())],
        "the DACL must hold one allow, for the current user"
    );
}

/// fails if the descriptor locks the shepherd out of its own pipe, or out of
/// the instance it creates after an accept, which needs
/// `FILE_CREATE_PIPE_INSTANCE`.
#[tokio::test]
async fn the_current_user_reaches_the_pipe_and_the_instance_made_after_an_accept() {
    let name = pipe_name("round-trip");
    let security = Arc::new(UserOnly::current_user().unwrap());
    let mut listener = Listener::bind_with(&name, security).unwrap();

    for round in 0_u8..2 {
        let (mut server, mut client) = tokio::time::timeout(BOUND, async {
            tokio::try_join!(listener.accept(), connect(&name))
        })
        .await
        .expect("a client of the same user must be admitted")
        .unwrap();
        client.write_all(&[round]).await.unwrap();
        let mut heard = [0_u8; 1];
        tokio::time::timeout(BOUND, server.read_exact(&mut heard))
            .await
            .expect("the byte the client wrote must arrive")
            .unwrap();
        assert_eq!(heard, [round]);
    }
}
