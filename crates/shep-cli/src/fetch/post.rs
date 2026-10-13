//! The POST half of [`crate::fetch`], behind the `otel` feature: one
//! [`post_json`], and the request builder and status reader only it uses.
//!
//! It reads no response body. A POST's answer is its status, and a
//! collector answering 204 sends no `Content-Length` at all, which is why
//! this does not ride [`crate::fetch::read_response`]: that reader is a GET's, and
//! it refuses a response it cannot size.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;

use super::{
    FetchError, MAX_HEADER_BYTES, Target, parse_status_line, peer_transport_error, tls_connector,
};

/// Posts `body` to `target` as `application/json`, bounded end to end by
/// `timeout`, and takes any 2xx as a success.
///
/// `headers` go out verbatim after the four this function builds. A value
/// carrying a control byte would end a header line early and let whatever
/// followed be read as another one, so a caller refuses those first: the
/// metrics dog's OTLP exporter does.
///
/// Unlike [`super::get`], this reads no body back. A POST's answer is its status,
/// and a collector answering 204 sends no `Content-Length` at all: reading
/// one the way [`super::get`] does would turn a working collector into a failed
/// export.
///
/// # Errors
/// Every [`FetchError`] variant except [`FetchError::Url`] and
/// [`FetchError::Redirect`] can come out of this call: a connect or TLS
/// failure, a malformed status line, a header block over
/// [`MAX_HEADER_BYTES`], a non-2xx status as [`FetchError::Status`], or the
/// deadline as [`FetchError::Timeout`]. A 3xx is refused as its status: this
/// is a POST, and following a redirect would resend a body to a host the
/// collector chose.
pub async fn post_json(
    target: &Target,
    headers: &[(String, String)],
    body: &[u8],
    timeout: Duration,
) -> Result<(), FetchError> {
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
) -> Result<(), FetchError> {
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

/// Writes `request`, flushes, then reads back the status. The flush matters
/// on the TLS branch for the reason [`crate::fetch::exchange`] gives for it.
async fn post_exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    request: &[u8],
) -> Result<(), FetchError> {
    stream.write_all(request).await?;
    stream.flush().await?;
    read_post_status(stream).await
}

/// Reads the status line and drains the header block, asking nothing of its
/// contents. A peer that closes the head without a blank line still answered
/// the status, which is the only thing a POST's caller acts on.
async fn read_post_status<S: AsyncRead + Unpin>(stream: S) -> Result<(), FetchError> {
    let mut head = BufReader::new(stream).take(MAX_HEADER_BYTES as u64);

    let mut status_line = String::new();
    head.read_line(&mut status_line).await?;
    let code = parse_status_line(&status_line)?;

    loop {
        let mut line = String::new();
        let read = head.read_line(&mut line).await?;
        if read == 0 {
            if head.limit() == 0 {
                return Err(FetchError::HeadersTooLarge {
                    limit: MAX_HEADER_BYTES,
                });
            }
            break;
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
    }

    if (200..300).contains(&code) {
        Ok(())
    } else {
        Err(FetchError::Status(code))
    }
}
