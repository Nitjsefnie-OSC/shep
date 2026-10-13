//! The POST half of [`crate::fetch`], behind the `otel` feature: one
//! [`post_json`], and the request builder and response reader only it uses.
//!
//! It reads the status, and the body the status's head sized, up to
//! [`crate::fetch::post::MAX_POST_BODY_BYTES`]. It does not ride
//! [`crate::fetch::read_response`]:
//! that reader is a GET's, and it refuses a response with no
//! `Content-Length`, which is how a collector answering 204 replies.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;

use super::{
    FetchError, MAX_HEADER_BYTES, Target, parse_status_line, peer_transport_error, tls_connector,
};

/// The most response body this reader keeps. A collector's OTLP answer is a
/// small JSON object; a peer sending more than this gets
/// [`FetchError::TooLarge`].
pub const MAX_POST_BODY_BYTES: usize = 8 * 1024;

/// What a POST's peer answered: a 2xx, and up to [`MAX_POST_BODY_BYTES`] of
/// the body it sent with it.
///
/// The body matters because a status alone is not what a collector says. An
/// OTLP collector that took part of a payload answers 200 and puts what it
/// dropped in the body, and a caller that never read it would report a clean
/// export every tick while points went missing.
#[derive(Debug, Default)]
pub struct PostReply {
    /// The response body, empty when the peer sent none. A 204 arrives with
    /// no body and no `Content-Length`.
    pub body: String,
}

/// Posts `body` to `target` as `application/json`, bounded end to end by
/// `timeout`, and takes any 2xx as an accepted export.
///
/// `headers` go out verbatim after the four this function builds. A value
/// carrying a control byte would end a header line early and let whatever
/// followed be read as another one, so a caller refuses those first: the
/// metrics dog's OTLP exporter does.
///
/// Unlike [`super::get`], a missing `Content-Length` is not an error here: it
/// is a 204. A 2xx whose body this could not size, or that came
/// `Transfer-Encoding: chunked`, is refused rather than guessed at, so a
/// caller never reads a partial answer as a whole one.
///
/// # Errors
/// Every [`FetchError`] variant except [`FetchError::Url`] and
/// [`FetchError::Redirect`] can come out of this call: a connect or TLS
/// failure, a malformed status line or header line, a header block over
/// [`MAX_HEADER_BYTES`], a `Transfer-Encoding` as [`FetchError::Chunked`], a
/// body over [`MAX_POST_BODY_BYTES`] as [`FetchError::TooLarge`], a peer that
/// closed the connection early as [`FetchError::Truncated`], a non-2xx status
/// as [`FetchError::Status`], or the deadline as [`FetchError::Timeout`]. A
/// 3xx is refused as its status: this is a POST, and following a redirect
/// would resend a body to a host the collector chose.
pub async fn post_json(
    target: &Target,
    headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
) -> Result<PostReply, FetchError> {
    match tokio::time::timeout(timeout, post_inner(target, headers, body)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(FetchError::Timeout),
    }
}

/// Connects to `target`, over TLS when `target.https`, and runs the POST
/// over whichever stream results.
async fn post_inner(
    target: &Target,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<PostReply, FetchError> {
    let request = build_post_request(target, headers, body);
    let tcp = TcpStream::connect((target.host.as_str(), target.port)).await?;
    if target.https {
        let domain = ServerName::try_from(target.host.clone())
            .map_err(|source| FetchError::Transport(std::io::Error::other(source)))?;
        let tls = tls_connector()
            .connect(domain, tcp)
            .await
            .map_err(peer_transport_error)?;
        post_exchange(tls, &request).await
    } else {
        post_exchange(tcp, &request).await
    }
}

/// The request line, headers and body [`post_inner`] sends, as the bytes to
/// write in one go. `Host` names the port only when it is off the scheme's
/// own default, and the connection is closed after the answer.
fn build_post_request(target: &Target, headers: &[(String, String)], body: &[u8]) -> Vec<u8> {
    let default_port = Target::default_port(target.https);
    let host = if target.port == default_port {
        target.host.clone()
    } else {
        format!("{}:{}", target.host, target.port)
    };
    let mut head = format!(
        "POST {} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        target.path,
        body.len(),
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut request = head.into_bytes();
    request.extend_from_slice(body);
    request
}

/// Writes `request`, flushes, then reads back the answer. The flush matters
/// on the TLS branch for the reason [`crate::fetch::exchange`] gives for it.
async fn post_exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    request: &[u8],
) -> Result<PostReply, FetchError> {
    stream.write_all(request).await?;
    stream.flush().await?;
    read_post_reply(stream).await
}

/// Reads the status line, the header block, and the body that head sized.
///
/// The header rules are [`crate::fetch::read_response`]'s, with the two
/// differences a POST needs: no `Location` to report, since a 3xx here is
/// refused as a status, and a missing `Content-Length` that means "no body"
/// rather than a malformed response.
async fn read_post_reply<S: AsyncRead + Unpin>(stream: S) -> Result<PostReply, FetchError> {
    // One budget for the status line and every header line together, spent
    // through a `Take`, for the reason [`crate::fetch::read_response`] gives
    // for it.
    let mut head = BufReader::new(stream).take(MAX_HEADER_BYTES as u64);

    let mut status_line = String::new();
    head.read_line(&mut status_line).await?;
    let code = parse_status_line(&status_line)?;

    let mut content_length: Option<u64> = None;
    // Set on an unparseable value or a second, disagreeing
    // `Content-Length`; the refusal is deferred to after the loop so it
    // keeps its place in the documented order.
    let mut content_length_ok = true;
    let mut transfer_encoding = false;

    loop {
        let mut line = String::new();
        let read = head.read_line(&mut line).await?;
        if read == 0 {
            if head.limit() == 0 {
                return Err(FetchError::HeadersTooLarge {
                    limit: MAX_HEADER_BYTES,
                });
            }
            return Err(FetchError::Transport(std::io::Error::other(
                "response headers ended without a blank line",
            )));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(FetchError::Transport(std::io::Error::other(
                "malformed header line: no colon",
            )));
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "transfer-encoding" => transfer_encoding = true,
            "content-length" => match value.parse::<u64>() {
                Ok(parsed) => match content_length {
                    Some(existing) if existing != parsed => content_length_ok = false,
                    Some(_) => {}
                    None => content_length = Some(parsed),
                },
                Err(_err) => content_length_ok = false,
            },
            _ => {}
        }
    }

    if !(200..300).contains(&code) {
        return Err(FetchError::Status(code));
    }
    if transfer_encoding {
        return Err(FetchError::Chunked);
    }
    if !content_length_ok {
        return Err(FetchError::Transport(std::io::Error::other(
            "content-length header was not a number, or two content-length headers disagreed",
        )));
    }
    let Some(content_length) = content_length else {
        return Ok(PostReply::default());
    };
    if content_length > MAX_POST_BODY_BYTES as u64 {
        return Err(FetchError::TooLarge {
            limit: MAX_POST_BODY_BYTES,
        });
    }
    // Safe: just checked `content_length <= MAX_POST_BODY_BYTES`, and that
    // constant is itself a `usize`, so `content_length` fits in one.
    let expected = content_length as usize;

    // Anything the `BufReader` read past the blank line is still in its
    // buffer, so unwrapping the `Take` resumes where the header loop stopped.
    let mut reader = head.into_inner();
    let mut body = vec![0u8; expected];
    let mut filled = 0;
    while filled < expected {
        let read = reader.read(&mut body[filled..]).await?;
        if read == 0 {
            return Err(FetchError::Truncated {
                expected,
                got: filled,
            });
        }
        filled += read;
    }

    Ok(PostReply {
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}
