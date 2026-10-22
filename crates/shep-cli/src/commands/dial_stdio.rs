//! `shep dial-stdio`: the remote end of the ssh transport.
//!
//! `ssh <dest> shep dial-stdio` runs this on the machine that holds the
//! shepherd, and the existing length-prefixed frames cross unchanged. The verb
//! copies bytes and nothing else: it does no handshake, no version check, and
//! never looks inside a frame.

use std::convert::Infallible;
use std::io;
use std::path::Path;

use shep_client::HANDSHAKE_TIMEOUT;
use shep_core::transport;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::cli::Format;
use crate::client::emit_error_locked;
use crate::exit::ExitCode;

/// Copies bytes between stdin/stdout and the shepherd at `socket`, then ends
/// the process.
///
/// Never returns. tokio's stdin read runs on a blocking thread that dropping
/// the runtime waits for, so returning an [`ExitCode`] would hang the process
/// on an open stdin after the shepherd side closed. Stdout is flushed by
/// [`bridge`] before this exits.
pub(crate) async fn dial_stdio(fmt: Format, socket: &Path) -> Infallible {
    let code = dial(fmt, socket, tokio::io::stdin(), tokio::io::stdout()).await;
    std::process::exit(i32::from(code as u8));
}

/// Connects to the shepherd at `socket` and runs [`bridge`] over it.
///
/// Nothing is written to `output` when no shepherd answers: the one error goes
/// to stderr.
async fn dial<I, O>(fmt: Format, socket: &Path, input: I, output: O) -> ExitCode
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let unreachable = |why: &dyn core::fmt::Display| {
        emit_error_locked(
            fmt,
            ExitCode::DaemonUnreachable,
            &format!("no shepherd answered at {}: {why}", socket.display()),
        );
        ExitCode::DaemonUnreachable
    };
    let stream = match tokio::time::timeout(HANDSHAKE_TIMEOUT, transport::connect(socket)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(err)) => return unreachable(&err),
        Err(_) => return unreachable(&"timed out"),
    };
    match bridge(input, output, stream).await {
        Ok(()) => ExitCode::Success,
        Err(err) => {
            emit_error_locked(fmt, ExitCode::Failure, &format!("connection lost: {err}"));
            ExitCode::Failure
        }
    }
}

/// Copies `input` to `shepherd` and `shepherd` to `output` until either side
/// reaches end of file or errors, then flushes `output`.
///
/// End of `input` closes the write half toward the shepherd, so the shepherd
/// sees an orderly end rather than a reset.
///
/// # Errors
///
/// The first I/O error on either direction, or on the final flush.
async fn bridge<I, O, S>(mut input: I, mut output: O, shepherd: S) -> io::Result<()>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    S: AsyncRead + AsyncWrite,
{
    let (mut from_shepherd, mut to_shepherd) = tokio::io::split(shepherd);
    let ended = {
        let upstream = async {
            tokio::io::copy(&mut input, &mut to_shepherd).await?;
            to_shepherd.shutdown().await
        };
        let downstream = async {
            tokio::io::copy(&mut from_shepherd, &mut output)
                .await
                .map(drop)
        };
        tokio::select! {
            ended = upstream => ended,
            ended = downstream => ended,
        }
    };
    ended.and(output.flush().await)
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use tokio::io::{AsyncReadExt, duplex};

    use super::*;

    /// Long enough for a loaded CI machine, short enough to fail as a named
    /// assertion rather than a hung run.
    const BOUND: Duration = Duration::from_secs(10);

    const BLOB_BYTES: usize = 4 * 1024 * 1024;

    /// Every byte value once, then a blob whose bytes come from a small LCG so
    /// a shifted or dropped run changes the comparison.
    fn payloads() -> [Vec<u8>; 2] {
        let every_value = (0..=u8::MAX).collect();
        let mut state = 0x9e37_79b9_u32;
        let blob = (0..BLOB_BYTES)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        [every_value, blob]
    }

    #[tokio::test]
    async fn bytes_cross_both_ways_unchanged() {
        for payload in payloads() {
            // stdin to the shepherd: the bridge ends on end of input.
            let (mut server, mut client) = transport::connected_pair().await.unwrap();
            let (mut stdin_tx, stdin) = duplex(64 * 1024);
            let (stdout, _stdout_rx) = duplex(64 * 1024);
            let (bridged, (), received) = tokio::time::timeout(BOUND, async {
                tokio::join!(
                    bridge(stdin, stdout, &mut client),
                    async {
                        stdin_tx.write_all(&payload).await.unwrap();
                        drop(stdin_tx);
                    },
                    async {
                        let mut received = Vec::new();
                        server.read_to_end(&mut received).await.unwrap();
                        received
                    },
                )
            })
            .await
            .expect("upstream copy finished");
            bridged.unwrap();
            assert!(received == payload, "stdin to shepherd altered the bytes");

            // The shepherd to stdout: the bridge ends when the shepherd closes.
            let (mut server, client) = transport::connected_pair().await.unwrap();
            let (_stdin_tx, stdin) = duplex(64 * 1024);
            let (stdout, mut stdout_rx) = duplex(64 * 1024);
            let (bridged, (), received) = tokio::time::timeout(BOUND, async {
                tokio::join!(
                    bridge(stdin, stdout, client),
                    async {
                        server.write_all(&payload).await.unwrap();
                        drop(server);
                    },
                    async {
                        let mut received = Vec::new();
                        stdout_rx.read_to_end(&mut received).await.unwrap();
                        received
                    },
                )
            })
            .await
            .expect("downstream copy finished");
            bridged.unwrap();
            assert!(received == payload, "shepherd to stdout altered the bytes");
        }
    }

    #[tokio::test]
    async fn the_bridge_ends_when_the_shepherd_side_closes_with_input_still_open() {
        let (server, client) = transport::connected_pair().await.unwrap();
        let (_stdin_tx, stdin) = duplex(1024);
        let (stdout, _stdout_rx) = duplex(1024);
        drop(server);

        let bridged = tokio::time::timeout(BOUND, bridge(stdin, stdout, client))
            .await
            .expect("the bridge outlived the shepherd side");
        bridged.unwrap();
    }

    #[tokio::test]
    async fn end_of_input_closes_the_write_half_toward_the_shepherd_and_ends_the_bridge() {
        let (mut server, mut client) = transport::connected_pair().await.unwrap();
        let (mut stdin_tx, stdin) = duplex(1024);
        let (stdout, _stdout_rx) = duplex(1024);
        stdin_tx.write_all(b"last words").await.unwrap();
        drop(stdin_tx);

        // `client` is lent, so it is still open when the bridge returns: the
        // shepherd's end of file below comes from the shutdown, not a drop.
        let bridged = tokio::time::timeout(BOUND, bridge(stdin, stdout, &mut client))
            .await
            .expect("the bridge outlived end of input");
        bridged.unwrap();

        let mut received = Vec::new();
        tokio::time::timeout(BOUND, server.read_to_end(&mut received))
            .await
            .expect("the shepherd never saw end of file")
            .unwrap();
        assert_eq!(received, b"last words");
    }

    #[tokio::test]
    async fn a_missing_socket_is_daemon_unreachable_and_writes_nothing_to_the_output_half() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("run").join("shep.sock");
        let (_stdin_tx, stdin) = duplex(1024);
        let (stdout, mut stdout_rx) = duplex(1024);

        let code = dial(Format::Table, &socket, stdin, stdout).await;

        assert_eq!(code, ExitCode::DaemonUnreachable);
        let mut written = Vec::new();
        stdout_rx.read_to_end(&mut written).await.unwrap();
        assert!(written.is_empty(), "stdout got {written:?}");
    }
}
