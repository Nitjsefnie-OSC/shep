//! `Request::TailLog`: the end of one sheep's log file, read on the host the
//! file is on.

use std::path::Path;

use shep_core::logtail::read_tail;
use shep_core::protocol::{LogStream, LogTail, ProcessInfo, Response, RpcError, RpcErrorCode};

use super::context::RpcContext;
use super::error::rpc_error;
use super::selector_verbs::not_found;

/// The tail of `stream`'s file for the sheep with `id`.
///
/// The file is the one the row records, never one the request names. A dog's
/// row counts.
///
/// # Errors
/// An id no row holds is [`RpcErrorCode::NotFound`]; a stopped engine is
/// whatever [`rpc_error`] makes of it. A file that is absent or unreadable
/// is an answer, not an error.
pub(super) async fn answer(
    id: u32,
    stream: LogStream,
    lines: u32,
    ctx: &RpcContext,
) -> Result<Response, RpcError> {
    let rows = ctx
        .supervisor
        .list_checked()
        .await
        .map_err(|err| rpc_error(&err))?;
    let row = rows.iter().find(|row| row.id == id).ok_or_else(not_found)?;
    tail_row(row, stream, lines).await.map(Response::LogTail)
}

/// The tail of `stream`'s file as `row` records it.
///
/// A relative path is opened as this process resolves it, against its own
/// directory, which is the file the log pump writes. Sees only what the pump
/// has flushed to the file.
///
/// # Errors
/// The blocking read task did not complete.
pub(super) async fn tail_row(
    row: &ProcessInfo,
    stream: LogStream,
    lines: u32,
) -> Result<LogTail, RpcError> {
    let recorded = match stream {
        LogStream::Out => &row.out_file,
        LogStream::Err => &row.err_file,
        _ => &None,
    };
    let Some(path) = recorded.clone() else {
        return Ok(LogTail::NoPath);
    };
    // `Some(false)` is the only value an app that stamps nothing reports.
    let stamped = row.log_timestamps.unwrap_or(true);
    let limit = usize::try_from(lines).unwrap_or(usize::MAX);
    // `std::fs`, so off the async workers.
    let read = tokio::task::spawn_blocking({
        let path = path.clone();
        move || read_tail(Path::new(&path), limit, stamped)
    })
    .await
    .map_err(|err| RpcError {
        code: RpcErrorCode::Internal,
        message: format!("the log read did not complete ({err})"),
        daemon_version: None,
    })?;
    Ok(match read {
        Ok((lines, truncated)) => LogTail::Lines { lines, truncated },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => LogTail::Missing { path },
        Err(err) => LogTail::Unreadable {
            path,
            reason: err.to_string(),
        },
    })
}
