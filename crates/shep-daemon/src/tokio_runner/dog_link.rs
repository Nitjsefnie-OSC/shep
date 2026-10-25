//! The socketpair a dog is spawned holding: the shepherd keeps one end and the
//! child's fd 4 is the other.

use std::os::fd::OwnedFd;

use command_fds::FdMapping;
use shep_core::dogs::DOG_LINK_FD;
use tokio::net::UnixStream;

use crate::runner::RunnerError;

/// Opens the pair, returning the shepherd's end and the mapping that puts the
/// other on [`DOG_LINK_FD`].
///
/// Blocking on the child's end for the reason the shepherd channel's is: the
/// child inherits `O_NONBLOCK` across the exec, and a plain blocking read in a
/// dog would get `EAGAIN`.
///
/// # Errors
///
/// - [`RunnerError::SpawnFailed`] if the socketpair cannot be made or
///   configured.
pub(super) fn pair() -> Result<(UnixStream, FdMapping), RunnerError> {
    let failed = |step: &str, error: std::io::Error| {
        RunnerError::SpawnFailed(format!("dog connection {step}: {error}"))
    };
    let (daemon_end, child_end) = UnixStream::pair().map_err(|e| failed("socketpair", e))?;
    let child_end = child_end.into_std().map_err(|e| failed("into_std", e))?;
    child_end
        .set_nonblocking(false)
        .map_err(|e| failed("set_nonblocking", e))?;
    Ok((
        daemon_end,
        FdMapping {
            parent_fd: OwnedFd::from(child_end),
            child_fd: DOG_LINK_FD,
        },
    ))
}
