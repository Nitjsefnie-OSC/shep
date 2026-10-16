//! Writing one reply, and logging it.
//!
//! [`respond`] is the only place a response head is written and the only
//! place the nosniff header is appended. [`reply`] is a head plus an
//! in-memory body, and it hands back the `(status, byte count)` pair the
//! access log line reports. [`log_access`] writes that line, escaping
//! every byte a client chose before it reaches `shep bleats`.

use std::net::SocketAddr;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::http::{self, Header, HttpError};

/// Writes exactly one response head. Every reply funnels through here,
/// and this is the one place the nosniff header is appended, since the
/// listing renders attacker-influenced filenames into HTML.
///
/// # Errors
/// [`HttpError`] if the write failed, or if a header value carried a
/// control byte (`write_head`'s response-splitting lock). The caller
/// answers with nothing further, since nothing has reached the stream.
pub(super) async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    content_length: u64,
    extra_headers: Vec<Header<'_>>,
) -> Result<(), HttpError> {
    let mut headers = extra_headers;
    headers.push(Header {
        name: "X-Content-Type-Options",
        value: "nosniff",
    });
    http::write_head(stream, status, content_type, content_length, &headers).await
}

/// [`respond`] plus the in-memory body every reply except a streamed file
/// or an empty redirect carries. Writes nothing further if the head itself
/// was refused.
async fn send(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: Vec<Header<'_>>,
) {
    if respond(stream, status, content_type, body.len() as u64, headers)
        .await
        .is_ok()
    {
        let _ = stream.write_all(body).await;
    }
}

/// [`send`] plus the `(status, byte count)` pair every in-memory reply
/// returns for [`super::handle_connection`]'s access-log line.
pub(super) async fn reply(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    headers: Vec<Header<'_>>,
) -> (u16, u64) {
    send(stream, status, content_type, body, headers).await;
    (status, body.len() as u64)
}

/// One access-log line per request, written to stdout: `serve` runs as a
/// registered sheep, so `shep bleats <name>` is this log. `path` is the
/// raw target or the resolved path, escaped here so no byte a client chose
/// reaches the operator's terminal.
pub(super) fn log_access(
    peer: Option<SocketAddr>,
    method: &str,
    path: &str,
    status: u16,
    bytes: u64,
) {
    let peer = peer.map_or_else(|| "-".to_string(), |addr| addr.to_string());
    let path = escape_for_log(path);
    println!("{peer} \"{method} {path}\" {status} {bytes}");
}

/// Escapes every byte outside printable ASCII (`0x20..=0x7e`) as `\xNN`,
/// so a byte a client chose never reaches an operator's terminal through
/// `shep bleats`.
fn escape_for_log(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        if (0x20..=0x7e).contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("\\x{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::escape_for_log;

    /// fails if a decoded C1 control byte reaches the log raw: `path::resolve`
    /// refuses control bytes below `0x20` and `0x7f`, so `%C2%9B` (CSI)
    /// resolves, and the access log is where it must be escaped.
    #[test]
    fn a_decoded_c1_control_byte_is_escaped_for_the_log() {
        assert_eq!(escape_for_log("/\u{9b}"), "/\\xC2\\x9B");
    }
}
