//! The fixture every dog verb's tests share.

use crate::cli::Format;
use crate::output::Streams;

/// A dog verb's streams under `--format table`, bare style; a test reading
/// a JSON field sets `fmt` itself.
pub(super) fn streams<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>) -> Streams<'a> {
    Streams {
        out,
        err,
        style: crate::style::Presentation::BARE,
        fmt: Format::Table,
    }
}
