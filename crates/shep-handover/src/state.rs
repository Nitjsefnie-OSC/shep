//! The state blob: a caller-defined type in a versioned envelope.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// The environment variable a handover leaves for its successor.
///
/// Its value is the old image's pid, a colon, and the path of the blob. The
/// pid keeps a child of the successor, which inherits the variable, from
/// mistaking itself for a successor. Distinct from the shepherd's own
/// `SHEP_HANDOVER`, so a dog the shepherd handed over never reads that one.
pub const HANDOVER_ENV: &str = "SHEP_PROCESS_HANDOVER";

/// A type that crosses a handover as the state blob.
///
/// The format number is the caller's: bump it when the type changes in a way
/// an older build could not read. A build refuses a blob it cannot read with
/// [`BootError::Format`](crate::BootError::Format), naming both numbers.
pub trait State: Serialize + DeserializeOwned {
    /// The format this build writes.
    const FORMAT: u32;

    /// Whether this build can read a blob written in `format`.
    ///
    /// Defaults to an exact match. Override it to read older formats too.
    #[must_use]
    fn reads(format: u32) -> bool {
        format == Self::FORMAT
    }
}
