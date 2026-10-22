//! A stand-in for `ssh` that runs the remote command on this machine against
//! a separate `$SHEP_HOME`, so the ssh transport is exercised end to end with
//! no sshd.
//!
//! Unix only: the stand-in is a `/bin/sh` script. The real hop, and the
//! Windows named-pipe leg behind it, are a human check.

use super::*;

#[cfg(unix)]
use shep_client::shep_core::protocol::{Request, Response};
#[cfg(unix)]
use shep_client::{Client, SshOptions};

#[cfg(unix)]
/// Writes an executable `ssh` into `dir` and returns its path.
///
/// It drops ssh's options and the destination, then runs the remote command
/// (`shep dial-stdio`) with `$SHEP_HOME` set to `remote_home` and the built
/// `shep` first on `PATH`. A caller can hand the path to
/// [`SshOptions::program`], or put `dir` first on a child's `PATH` so the
/// child finds it as `ssh`.
pub(crate) fn write_ssh_stand_in(dir: &Path, remote_home: &Path) -> PathBuf {
    let shep = std::process::Command::cargo_bin("shep").unwrap();
    let shep_dir = Path::new(shep.get_program()).parent().unwrap().to_owned();
    let script = format!(
        "#!/bin/sh\n\
         while [ \"$#\" -gt 0 ]; do\n\
         \x20 case \"$1\" in\n\
         \x20   -T) shift ;;\n\
         \x20   -o) shift 2 ;;\n\
         \x20   --) shift; break ;;\n\
         \x20   *) break ;;\n\
         \x20 esac\n\
         done\n\
         shift\n\
         export PATH='{}':\"$PATH\" SHEP_HOME='{}'\n\
         exec \"$@\"\n",
        shep_dir.display(),
        remote_home.display(),
    );
    let path = dir.join("ssh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// The names a `ListFlock` answered, dogs included.
#[cfg(unix)]
fn names(response: Response) -> Vec<String> {
    let Response::Flock(flock) = response else {
        panic!("ListFlock was not answered with a flock: {response:?}");
    };
    flock.into_iter().map(|info| info.name).collect()
}

/// A shepherd on the remote home answers through the stand-in, and the local
/// home's own shepherd, running at the same time, is not the one reached.
#[cfg(unix)]
#[tokio::test]
async fn connect_ssh_through_a_stand_in_reaches_a_real_shepherd() {
    let local = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let mut guard = DaemonGuard::default();
    for (home, script) in [(&local, "local-sheep.sh"), (&remote, "remote-sheep.sh")] {
        let script = write_script(
            home,
            script,
            &format!("{}{}", script_header(), sleep_line(SCRIPT_SLEEP_SECS)),
        );
        let started = shep(home.path())
            .arg("start")
            .arg(&script)
            .output()
            .unwrap();
        guard.adopt_home(home.path());
        assert_success(&started);
    }
    let program = write_ssh_stand_in(bin.path(), remote.path());

    let client = Client::connect_ssh_with(
        "build-01",
        &SshOptions::default()
            .program(program)
            .timeout(Some(CMD_TIMEOUT)),
    )
    .await
    .unwrap();
    let flock = client.request(Request::ListFlock).await.unwrap();

    assert_eq!(client.host(), Some("build-01"));
    let flock = names(flock);
    assert!(flock.contains(&"remote-sheep".to_owned()), "{flock:?}");
    assert!(!flock.contains(&"local-sheep".to_owned()), "{flock:?}");
    drop(client);
    graceful_kill(local.path());
    graceful_kill(remote.path());
}
