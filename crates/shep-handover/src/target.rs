//! Choosing the binary a handover execs.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Where this process's binary was when it started, as `argv[0]` resolved
/// against the startup directory.
///
/// The inner `Option` is "recorded, and there was nothing usable to record",
/// distinct from "never recorded", which is a missing call.
static LAUNCH_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Record how this process was invoked, for a later [`exec_target`].
///
/// Call once, before anything can move the current directory: it resolves
/// `argv[0]` against it. The first call wins.
pub fn record_launch_path() {
    let _ = LAUNCH_PATH.set(launch_path_from_argv());
}

/// The binary to exec for a handover, and never the running image.
///
/// Prefers the path [`record_launch_path`] recorded, falling back to
/// [`std::env::current_exe`]. On Linux `current_exe` reads `/proc/self/exe`,
/// which after an upgrade renames a new binary over the old one comes back as
/// `"<path> (deleted)"` and cannot be exec'd.
///
/// # Errors
///
/// [`io::ErrorKind::NotFound`] if neither candidate is a file safe to exec.
/// The message names each candidate tried and what was wrong with it.
pub fn exec_target() -> io::Result<PathBuf> {
    let recorded = LAUNCH_PATH.get().cloned().flatten();
    let current = std::env::current_exe();
    resolve(
        [recorded, current.as_deref().ok().map(Path::to_path_buf)],
        current.as_ref().err(),
    )
}

/// The first of `candidates` that [`check`] accepts, skipping a `None`.
/// `current_exe_error` is folded into the diagnostic only.
fn resolve(
    candidates: [Option<PathBuf>; 2],
    current_exe_error: Option<&io::Error>,
) -> io::Result<PathBuf> {
    let mut refusals = Vec::new();
    for candidate in candidates.into_iter().flatten() {
        match check(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(problem) => refusals.push(format!("{} ({problem})", candidate.display())),
        }
    }
    if let Some(e) = current_exe_error {
        refusals.push(format!("this process's own image ({e})"));
    }
    if refusals.is_empty() {
        refusals.push("no candidate at all".to_owned());
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("no binary to exec: {}", refusals.join("; ")),
    ))
}

/// Why a candidate path is not safe to exec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    /// The path carries Linux's `" (deleted)"` suffix, so it names an
    /// unlinked inode rather than a file.
    DeletedInode,
    /// Nothing is at the path, or it could not be read.
    Missing,
    /// Something is at the path, but it is a directory or a device.
    NotAFile,
}

impl core::fmt::Display for Problem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::DeletedInode => "names a deleted inode, not a file",
            Self::Missing => "is not on disk",
            Self::NotAFile => "is not a file",
        })
    }
}

/// Whether `candidate` is a file a process may replace itself with.
///
/// The `" (deleted)"` check runs first and refuses even a path that really
/// does exist.
fn check(candidate: &Path) -> Result<(), Problem> {
    if candidate.to_string_lossy().contains(" (deleted)") {
        return Err(Problem::DeletedInode);
    }
    match std::fs::metadata(candidate) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(Problem::NotAFile),
        Err(_) => Err(Problem::Missing),
    }
}

#[cfg(unix)]
/// [`check`] for a path the caller named, as the error a handover returns.
///
/// # Errors
///
/// [`io::ErrorKind::NotFound`] if `path` is not a file safe to exec.
pub(crate) fn vet(path: &Path) -> io::Result<()> {
    check(path).map_err(|problem| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no binary to exec: {} ({problem})", path.display()),
        )
    })
}

/// This process's `argv[0]`, resolved against the current directory.
///
/// `None` when there is no `argv[0]`, when it is empty, or when it holds no
/// separator and so came from a `PATH` lookup this cannot undo. An absolute
/// `argv[0]` passes through the join unchanged.
fn launch_path_from_argv() -> Option<PathBuf> {
    let argv0 = PathBuf::from(std::env::args_os().next()?);
    if argv0.as_os_str().is_empty() {
        return None;
    }
    if argv0.is_absolute() {
        return Some(argv0);
    }
    let has_separator = argv0
        .parent()
        .is_some_and(|dir| !dir.as_os_str().is_empty());
    has_separator.then(|| std::env::current_dir().ok().map(|cwd| cwd.join(&argv0)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_exec_target_is_a_file() {
        let path = exec_target().unwrap();
        assert!(path.is_file(), "{}", path.display());
    }

    #[test]
    fn a_deleted_inode_candidate_is_refused_with_its_reason() {
        let deleted = PathBuf::from("/opt/app/app (deleted)");
        let err = resolve([None, Some(deleted)], None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "no binary to exec: /opt/app/app (deleted) (names a deleted inode, not a file)"
        );
    }

    #[test]
    fn a_deleted_suffix_is_refused_even_when_the_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app (deleted)");
        std::fs::write(&path, "really on disk").unwrap();
        assert_eq!(check(&path), Err(Problem::DeletedInode));
    }

    #[test]
    fn a_missing_candidate_and_a_directory_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            check(&dir.path().join("never-written")),
            Err(Problem::Missing)
        );
        assert_eq!(check(dir.path()), Err(Problem::NotAFile));
    }

    #[test]
    fn the_first_usable_candidate_wins() {
        let me = std::env::current_exe().unwrap();
        let skipped = PathBuf::from("/nonexistent/app");
        assert_eq!(
            resolve([Some(skipped), Some(me.clone())], None).unwrap(),
            me
        );
    }

    #[test]
    fn no_candidate_at_all_says_so() {
        let err = resolve([None, None], None).unwrap_err();
        assert_eq!(err.to_string(), "no binary to exec: no candidate at all");
    }

    #[test]
    fn argv0_resolves_against_the_startup_directory() {
        // The harness is invoked by a path with a separator, so this proves
        // the argv[0] arm reaches a real file rather than that the join is right.
        let path = launch_path_from_argv().expect("argv[0] names a path");
        assert!(path.is_file(), "{}", path.display());
    }
}
