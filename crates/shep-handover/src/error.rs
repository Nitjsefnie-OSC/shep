//! Why a handover did not happen, and why a boot found nothing to adopt.

use std::io;

/// Why [`Handover::exec`](crate::Handover::exec) returned.
///
/// It returns only when the handover did not happen, so every variant means
/// the old image is still running with every descriptor it had, close-on-exec
/// again, and no blob left on disk. The caller falls back to a stop and a
/// start.
#[derive(Debug)]
#[non_exhaustive]
pub enum HandOverError {
    /// This platform has no `execve`.
    Unsupported,
    /// No binary is safe to exec: none was named and this process's own image
    /// is not usable, or the one named is not a file.
    Target(io::Error),
    /// The [`Fitness`](crate::Fitness) gate refused the handover.
    Refused {
        /// The gate's reason.
        reason: String,
    },
    /// Two descriptors were carried under one key.
    DuplicateKey {
        /// The key named twice.
        key: String,
    },
    /// The state does not serialize.
    State(serde_json::Error),
    /// The blob could not be written.
    Blob(io::Error),
    /// A descriptor to carry could not be kept across the exec, which is what
    /// one that is not open looks like.
    Descriptor {
        /// The key the descriptor was carried under.
        key: String,
        /// What `fcntl` said.
        source: io::Error,
    },
    /// An argument or an environment entry holds an interior NUL.
    Argument(io::Error),
    /// The `execve` itself failed.
    Exec(io::Error),
}

impl core::fmt::Display for HandOverError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("handing over is not supported on this platform"),
            Self::Target(err) => write!(f, "the handover has no binary to exec: {err}"),
            Self::Refused { reason } => write!(f, "the handover was refused: {reason}"),
            Self::DuplicateKey { key } => {
                write!(f, "two descriptors were carried under the key '{key}'")
            }
            Self::State(err) => write!(f, "the handover state does not serialize: {err}"),
            Self::Blob(err) => write!(f, "the handover blob could not be written: {err}"),
            Self::Descriptor { key, source } => write!(
                f,
                "the descriptor carried as '{key}' could not be kept across the exec: {source}"
            ),
            Self::Argument(err) => write!(f, "the handover cannot pass an argument: {err}"),
            Self::Exec(err) => write!(f, "the exec failed: {err}"),
        }
    }
}

impl core::error::Error for HandOverError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Target(err) | Self::Blob(err) | Self::Argument(err) | Self::Exec(err) => {
                Some(err)
            }
            Self::Descriptor { source, .. } => Some(source),
            Self::State(err) => Some(err),
            Self::Unsupported | Self::Refused { .. } | Self::DuplicateKey { .. } => None,
        }
    }
}

/// Why [`boot`](crate::boot) found a handover it could not adopt.
///
/// Every variant means the predecessor is already gone, so there is nothing to
/// hand back to. The blob is removed and every descriptor it carried is
/// closed; the caller starts fresh or exits, whichever is safe for what it
/// supervises.
#[derive(Debug)]
#[non_exhaustive]
pub enum BootError {
    /// This platform has no `execve`, so no image is a successor.
    Unsupported,
    /// [`HANDOVER_ENV`](crate::HANDOVER_ENV) is set, and not to a pid and a
    /// path.
    Marker {
        /// What the variable held.
        value: String,
    },
    /// The blob could not be read.
    Blob(io::Error),
    /// The blob is not an envelope this build wrote.
    Malformed(serde_json::Error),
    /// The envelope around the state is a layout this build does not
    /// implement.
    Envelope {
        /// The layout the blob names, if it names one.
        found: Option<u64>,
    },
    /// The state is in a format this build cannot read.
    Format {
        /// The format the blob names, if it names one.
        found: Option<u64>,
        /// The format this build reads.
        supported: u32,
    },
    /// The state is in a format this build reads, and does not fit its type.
    State(serde_json::Error),
    /// A descriptor the blob names is not an inherited one.
    Descriptor {
        /// The key it was carried under.
        key: String,
        /// The number the blob gives it.
        number: i32,
        /// Why it cannot be adopted.
        source: io::Error,
    },
}

impl core::fmt::Display for BootError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("handing over is not supported on this platform"),
            Self::Marker { value } => write!(
                f,
                "{} is set to '{value}', which is not a pid and a path",
                crate::HANDOVER_ENV
            ),
            Self::Blob(err) => write!(f, "the handover blob could not be read: {err}"),
            Self::Malformed(err) => write!(f, "the handover blob is not readable: {err}"),
            Self::Envelope { found: Some(found) } => write!(
                f,
                "the handover blob is envelope layout {found}, which this build does not implement"
            ),
            Self::Envelope { found: None } => {
                f.write_str("the handover blob names no envelope layout")
            }
            Self::Format {
                found: Some(found),
                supported,
            } => write!(
                f,
                "the handover state is format {found}, and this build reads format {supported}"
            ),
            Self::Format {
                found: None,
                supported,
            } => write!(
                f,
                "the handover state names no format, and this build reads format {supported}"
            ),
            Self::State(err) => write!(f, "the handover state is not readable: {err}"),
            Self::Descriptor {
                key,
                number,
                source,
            } => write!(
                f,
                "the descriptor carried as '{key}' (number {number}) cannot be adopted: {source}"
            ),
        }
    }
}

impl core::error::Error for BootError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Blob(err) => Some(err),
            Self::Malformed(err) | Self::State(err) => Some(err),
            Self::Descriptor { source, .. } => Some(source),
            Self::Unsupported
            | Self::Marker { .. }
            | Self::Envelope { .. }
            | Self::Format { .. } => None,
        }
    }
}
