//! Reading the end of one sheep's log file.
//!
//! Shared between `shep bleats --no-follow`, the `whistle` tool and the
//! shepherd's `Request::TailLog` answer, so a tail read on this host and a
//! tail read on a remote one cut a file the same way.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// The most of one log file a tail will read to find the lines it wants.
///
/// Binds only when lines average over 5 KiB, so in ordinary use the caller's
/// line count is the bound that decides. A line count alone cannot bound
/// memory: one arbitrarily long line with no newline would defeat it.
pub const TAIL_WINDOW_BYTES: u64 = 256 * 1024;

/// The last `limit` lines of one log file, bounded twice: a
/// [`TAIL_WINDOW_BYTES`] window from the end of the file, then `limit`
/// lines within it. Returns the lines and whether either bound cut them
/// short.
///
/// `std::fs`, not `tokio::fs`: the callers' tokio builds have no `fs`
/// feature. Each line loses its daemon-added timestamp
/// ([`crate::logstamp`]) when `stamped` says the shepherd added one, so it
/// reads the same as a line from the bus. A non-zero seek discards bytes up
/// to the first `\n`, rather than rendering a mid-line fragment.
///
/// `stamped` is a row's `log_timestamps` read the absent way: an app that
/// turned the stamp off wrote its own prefix, and cutting 30 bytes off an
/// RFC 3339 line of its own cuts into the app's text.
///
/// # Errors
/// The file could not be opened, `stat`ed, seeked, or read.
pub fn read_tail(path: &Path, limit: usize, stamped: bool) -> io::Result<(Vec<String>, bool)> {
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TAIL_WINDOW_BYTES);
    // `start > 0` means the byte window itself left content behind, before
    // a single line has been counted: the file is bigger than the window.
    let window_truncated = start > 0;
    if start > 0 {
        file.seek(SeekFrom::Start(start))?;
    }

    let mut window = Vec::new();
    file.read_to_end(&mut window)?;

    let window: &[u8] = if start > 0 {
        match window.iter().position(|&b| b == b'\n') {
            Some(newline) => &window[newline + 1..],
            None => &[],
        }
    } else {
        &window
    };

    let text = String::from_utf8_lossy(window);
    // The daemon's per-line stamp comes off here, so a `line` has one
    // meaning across both of `bleats`' paths: the follow path reads the
    // bus, which carries a sheep's own bytes. The stamp stays in the file
    // for `tail`, `less` and `grep`. Only a row the shepherd stamped has a
    // stamp to take off.
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            if stamped {
                crate::logstamp::strip(line)
            } else {
                line
            }
        })
        .map(str::to_string)
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let keep_from = lines.len().saturating_sub(limit);
    let truncated = window_truncated || keep_from > 0;
    lines.drain(..keep_from);
    Ok((lines, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shepherd's stamp spelled out, so the fixture does not come from
    /// the code under test.
    const STAMP: &str = "2026-09-02T14:22:31.412+02:00 ";

    fn file_with(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("web-out.log");
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn the_last_n_lines_are_returned() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_with(dir.path(), "one\ntwo\nthree\nfour\n");

        let (lines, truncated) = read_tail(&path, 2, false).unwrap();

        assert_eq!(lines, vec!["three".to_string(), "four".to_string()]);
        assert!(truncated, "two lines were left behind");
        let (all, truncated) = read_tail(&path, 10, false).unwrap();
        assert_eq!(all.len(), 4);
        assert!(!truncated, "the whole file fits");
    }

    #[test]
    fn a_file_larger_than_the_window_reports_truncated_and_drops_the_partial_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let filler = "x".repeat(usize::try_from(TAIL_WINDOW_BYTES).unwrap() + 1024);
        let path = file_with(dir.path(), &format!("{filler}\nshort\n"));

        let (lines, truncated) = read_tail(&path, 50, false).unwrap();

        assert_eq!(lines, vec!["short".to_string()]);
        assert!(truncated);
    }

    #[test]
    fn a_stamped_line_loses_its_stamp_and_an_unstamped_one_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_with(
            dir.path(),
            &format!("{STAMP}stamped\nno stamp on this one\n"),
        );

        let (stripped, _) = read_tail(&path, 10, true).unwrap();
        assert_eq!(
            stripped,
            vec!["stamped".to_string(), "no stamp on this one".to_string()]
        );
        let (kept, _) = read_tail(&path, 10, false).unwrap();
        assert_eq!(kept[0], format!("{STAMP}stamped"));
    }

    #[test]
    fn a_missing_file_is_an_error_of_kind_not_found() {
        let dir = tempfile::tempdir().unwrap();

        let err = read_tail(&dir.path().join("absent.log"), 10, true).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
