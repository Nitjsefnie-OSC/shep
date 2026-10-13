//! The `execve` itself, and putting everything back when it fails.

use core::convert::Infallible;
use std::ffi::{CString, OsString};
use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::os::fd::RawFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::HandOverError;
use crate::probe::PROBE_ENV;
use crate::state::HANDOVER_ENV;
use crate::sys;

/// Everything a handover has decided, ready to be done.
pub(crate) struct Plan {
    /// The binary to exec.
    pub target: PathBuf,
    /// The whole argument vector, `argv[0]` first.
    pub argv: Vec<OsString>,
    /// The descriptors to carry, by key.
    pub fds: Vec<(String, RawFd)>,
    /// The encoded state blob.
    pub blob: Vec<u8>,
    /// The directory the blob is written into.
    pub dir: PathBuf,
}

/// Write the blob, clear `FD_CLOEXEC` on the carried descriptors and only
/// those, and become `plan.target`.
///
/// Ordered so that everything which can fail without side effects does so
/// first: the argument vector and the environment are built before anything
/// is written or cleared.
///
/// # Errors
///
/// The blob could not be written, an argument holds an interior NUL, a
/// descriptor is not open, or the exec failed. Each returns with no blob on
/// disk and `FD_CLOEXEC` back on every descriptor it cleared.
pub(crate) fn run(plan: &Plan) -> Result<Infallible, HandOverError> {
    let path = c_string(plan.target.as_os_str().as_bytes())?;
    let argv = plan
        .argv
        .iter()
        .map(|arg| c_string(arg.as_bytes()))
        .collect::<Result<Vec<_>, _>>()?;
    let blob_path = blob_path(&plan.dir);
    let env = successor_env(&blob_path)?;

    write_blob(&blob_path, &plan.blob).map_err(HandOverError::Blob)?;

    let mut cleared = Vec::new();
    let failure = keep_and_exec(plan, &path, &argv, &env, &mut cleared);
    // Put every descriptor back and the blob away: the caller falls back to a
    // stop and a start with this image still running, and a later spawn would
    // inherit what was left clear.
    for fd in cleared {
        let _ = sys::close_on_exec(fd);
    }
    let _ = std::fs::remove_file(&blob_path);
    match failure {
        Ok(never) => match never {},
        Err(err) => Err(err),
    }
}

/// [`run`]'s clear-and-exec, recording what it cleared as it goes.
///
/// `cleared` is pushed to only when a descriptor was close-on-exec and is not
/// any more, so it never names one this process did not change: a descriptor
/// inherited without the bit keeps it clear after a failed exec.
fn keep_and_exec(
    plan: &Plan,
    path: &CString,
    argv: &[CString],
    env: &[CString],
    cleared: &mut Vec<RawFd>,
) -> Result<Infallible, HandOverError> {
    for (key, fd) in &plan.fds {
        let changed = sys::keep_across_exec(*fd).map_err(|source| HandOverError::Descriptor {
            key: key.clone(),
            source,
        })?;
        if changed {
            cleared.push(*fd);
        }
    }
    // `execve` rather than `execv`: `execv` inherits this process's `environ`,
    // so pointing the successor at the blob would need `std::env::set_var`,
    // which is unsafe and unsound with other threads running.
    nix::unistd::execve(path, argv, env)
        .map_err(|errno| HandOverError::Exec(io::Error::from(errno)))
}

/// A path in `dir` no other handover will pick: it names this process, the
/// time and a counter. The pid alone is not enough, since an exec keeps it and
/// a successor that never read its blob would meet the file again.
fn blob_path(dir: &Path) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    dir.join(format!(
        "shep-handover-{}-{nanos}-{n}.json",
        std::process::id()
    ))
}

/// Create `path` at mode `0600`, since the state may carry a secret, and a
/// later `chmod` leaves a world-readable window.
fn write_blob(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let written = file.write_all(bytes);
    if written.is_err() {
        let _ = std::fs::remove_file(path);
    }
    written
}

/// This process's environment, with [`HANDOVER_ENV`] set to this pid and
/// `blob_path`.
///
/// Any inherited value of that variable is dropped: a stale entry from an
/// earlier handover names a file that has already been read and unlinked. So is
/// the probe variable, which would make the successor answer a question nobody
/// asked.
fn successor_env(blob_path: &Path) -> Result<Vec<CString>, HandOverError> {
    let mut env = std::env::vars_os()
        .filter(|(name, _)| name != HANDOVER_ENV && name != PROBE_ENV)
        .map(|(name, value)| {
            let mut entry = name.into_vec();
            entry.push(b'=');
            entry.extend(value.into_vec());
            c_string(&entry)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut marker = format!("{HANDOVER_ENV}={}:", std::process::id()).into_bytes();
    marker.extend(blob_path.as_os_str().as_bytes());
    env.push(c_string(&marker)?);
    Ok(env)
}

fn c_string(bytes: &[u8]) -> Result<CString, HandOverError> {
    CString::new(bytes).map_err(|err| HandOverError::Argument(io::Error::other(err)))
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd as _;

    use super::*;

    fn plan(dir: &Path, fds: Vec<(String, RawFd)>) -> Plan {
        Plan {
            target: std::env::current_exe().unwrap(),
            argv: vec![OsString::from("app")],
            fds,
            blob: b"{}".to_vec(),
            dir: dir.to_path_buf(),
        }
    }

    #[test]
    fn a_descriptor_that_is_not_open_unwinds_the_ones_cleared_before_it() {
        let dir = tempfile::tempdir().unwrap();
        let (read, _write) = std::io::pipe().unwrap();
        let fds = vec![
            ("open".to_owned(), read.as_raw_fd()),
            ("gone".to_owned(), 1_000_000),
        ];

        let err = run(&plan(dir.path(), fds)).unwrap_err();

        assert!(
            matches!(&err, HandOverError::Descriptor { key, .. } if key == "gone"),
            "{err}"
        );
        assert!(sys::is_close_on_exec(read.as_raw_fd()).unwrap());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "no blob left"
        );
    }

    #[test]
    fn a_failed_exec_restores_only_the_bits_it_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let not_executable = dir.path().join("not-a-binary");
        std::fs::write(&not_executable, "this will never execute").unwrap();
        let (cleared_by_us, _w1) = std::io::pipe().unwrap();
        let (inherited_clear, _w2) = std::io::pipe().unwrap();
        sys::keep_across_exec(inherited_clear.as_raw_fd()).unwrap();
        let mut failing = plan(
            dir.path(),
            vec![
                ("ours".to_owned(), cleared_by_us.as_raw_fd()),
                ("theirs".to_owned(), inherited_clear.as_raw_fd()),
            ],
        );
        failing.target = not_executable;

        let err = run(&failing).unwrap_err();

        assert!(matches!(err, HandOverError::Exec(_)), "{err}");
        assert!(sys::is_close_on_exec(cleared_by_us.as_raw_fd()).unwrap());
        assert!(
            !sys::is_close_on_exec(inherited_clear.as_raw_fd()).unwrap(),
            "a descriptor inherited without the bit is left as it came"
        );
    }

    #[test]
    fn an_argument_with_an_interior_nul_is_refused_before_anything_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut bad = plan(dir.path(), Vec::new());
        bad.argv.push(OsString::from("a\0b"));

        let err = run(&bad).unwrap_err();

        assert!(matches!(err, HandOverError::Argument(_)), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_blob_is_private_to_its_owner() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().unwrap();
        let path = blob_path(dir.path());
        write_blob(&path, b"{}").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn two_blobs_in_one_directory_never_share_a_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_ne!(blob_path(dir.path()), blob_path(dir.path()));
    }

    #[test]
    fn the_successor_env_never_carries_a_probe() {
        let env = successor_env(Path::new("/tmp/blob.json")).unwrap();
        assert!(
            env.iter()
                .all(|e| !e.to_bytes().starts_with(PROBE_ENV.as_bytes()))
        );
    }

    #[test]
    fn the_successor_env_replaces_a_stale_marker_with_this_pid_and_the_blob() {
        let env = successor_env(Path::new("/tmp/blob.json")).unwrap();
        let markers: Vec<_> = env
            .iter()
            .filter(|e| e.to_bytes().starts_with(HANDOVER_ENV.as_bytes()))
            .collect();
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].to_str().unwrap(),
            format!("{HANDOVER_ENV}={}:/tmp/blob.json", std::process::id())
        );
    }
}
