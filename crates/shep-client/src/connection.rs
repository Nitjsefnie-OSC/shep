//! The bounded connect-plus-handshake: [`Connection::open`], [`ConnectError`]
//!
//! A bound-but-not-accepting unix socket still completes `connect(2)` into
//! the kernel backlog, so a bare connect is not a readiness probe. Only a
//! completed version handshake (connect, send [`Hello`], receive a
//! [`HelloAck`]) means the daemon is up. [`Connection::open`] bounds the
//! whole attempt in one [`tokio::time::timeout`].
//!
//! The OS transport is [`shep_core::transport`]'s to pick: a Windows named
//! pipe has the same readiness trap when every server instance is busy.

use core::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use shep_core::destination::DestinationError;
use shep_core::protocol::{
    Hello, HelloAck, HelloReply, PROTOCOL_VERSION, WireError, codec, decode_frame, encode_frame,
};
use shep_core::transport;

use crate::ssh::SshCause;

/// Budget for one connect-plus-handshake attempt.
///
/// Mirrors the daemon's own `HANDSHAKE_TIMEOUT_MS = 5_000`
/// (`shep-daemon/src/server.rs`) so neither side out-waits the other. The
/// `spawn` module's own connect-and-wait budget reads from this constant too.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Any byte stream a [`Connection`] can handshake and run over: the OS
/// socket, or a pipe to an ssh child.
pub(crate) trait ByteStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> ByteStream for T {}

/// The framed transport a [`Connection`] wraps, named so callers past this
/// module don't have to spell out the framed type themselves.
///
/// Boxed rather than generic: `Connection`, the actor and `Client` stay
/// non-generic, so the actor compiles once, and one vtable call per read is
/// noise beside JSON decoding. This alias also keeps `crate::actor` free of
/// any platform gate.
pub(crate) type Frames = Framed<Box<dyn ByteStream>, LengthDelimitedCodec>;

/// Why `Connection::open` failed.
///
/// Non-exhaustive: expect more variants.
#[derive(Debug)]
#[non_exhaustive]
pub enum ConnectError {
    /// `connect(2)` itself failed: nothing is listening at `path` (no
    /// socket file, permission denied, connection refused, ...).
    Connect {
        /// The socket path that was dialed.
        path: PathBuf,
        /// The OS error `connect` returned.
        source: std::io::Error,
    },
    /// A framed read or write failed after the connection was established.
    Io(std::io::Error),
    /// The `Hello` failed to encode, or the reply frame failed to decode.
    Wire(WireError),
    /// The peer closed the connection before a [`HelloReply`] arrived.
    HandshakeClosed,
    /// Connect succeeded but no [`HelloReply`] (and no close) arrived within
    /// the timeout. Distinct from [`Self::Connect`]: a refusal means nothing
    /// is listening, a timeout means something is bound but not answering.
    HandshakeTimeout {
        /// The timeout that was exceeded.
        after: Duration,
    },
    /// The daemon refused the handshake on protocol-version skew. `client`
    /// is this client's own [`PROTOCOL_VERSION`]; `message` is the
    /// daemon's own sentence, not meant for parsing.
    ProtocolMismatch {
        /// This client's own protocol version.
        client: u32,
        /// The daemon's own crate version, when it named one.
        ///
        /// `None` from a daemon built before this field existed. Read it
        /// as unknown rather than assuming either side is the older build.
        daemon_version: Option<String>,
        /// The daemon's refusal message, verbatim.
        message: String,
    },
    /// The destination handed to [`Client::connect_ssh_with`] is not one
    /// [`Destination::parse`] accepts. Nothing was spawned.
    ///
    /// [`Client::connect_ssh_with`]: crate::Client::connect_ssh_with
    /// [`Destination::parse`]: shep_core::destination::Destination::parse
    Destination(DestinationError),
    /// Dialling `destination` through ssh failed; `cause` says how.
    Ssh {
        /// The destination that was dialled.
        destination: String,
        /// What went wrong, on this side or the far one.
        cause: SshCause,
    },
}

impl ConnectError {
    /// The [`shep_core::exit`] code a process stopping on this error uses:
    /// a refusal is a protocol mismatch, a destination ssh would misread is a
    /// usage error, and every other stage is nothing usable answering.
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        use shep_core::exit;
        match self {
            Self::ProtocolMismatch { .. } => exit::PROTOCOL_MISMATCH,
            Self::Destination(_) => exit::USAGE,
            Self::Connect { .. }
            | Self::Ssh { .. }
            | Self::Io(_)
            | Self::Wire(_)
            | Self::HandshakeClosed
            | Self::HandshakeTimeout { .. } => exit::DAEMON_UNREACHABLE,
        }
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect { path, source } => {
                write!(f, "could not connect to `{}`: {source}", path.display())
            }
            Self::Io(err) => write!(f, "connection I/O error: {err}"),
            Self::Wire(err) => write!(f, "handshake frame error: {err}"),
            Self::HandshakeClosed => {
                f.write_str("the daemon closed the connection during the handshake")
            }
            Self::HandshakeTimeout { after } => {
                write!(f, "the handshake did not complete within {after:?}")
            }
            Self::Destination(err) => write!(f, "invalid destination: {err}"),
            Self::Ssh { destination, cause } => cause.write_for(f, destination),
            // `message` states the daemon's side of the skew; this arm adds
            // what it cannot: what to do, and which shep to do it against.
            // Both directions of the remedy are named, since `client` and
            // `message` don't say which build is the older one.
            Self::ProtocolMismatch {
                client,
                daemon_version,
                message,
            } => {
                let shep = daemon_version
                    .as_deref()
                    .map_or_else(|| "the running shep".to_string(), |v| format!("shep {v}"));
                write!(
                    f,
                    "protocol mismatch (this client speaks {client}): {message}. \
                     The older of the two has to be replaced: rebuild or reinstall this \
                     program against {shep} and restart it, or upgrade shep itself and run \
                     `shep daemon reload`"
                )
            }
        }
    }
}

impl core::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Connect { source, .. } | Self::Io(source) => Some(source),
            Self::Wire(err) => Some(err),
            Self::Destination(err) => Some(err),
            // the cause's own source, not the cause: `Display` already
            // spelled the cause out, and a chain would print it twice
            Self::Ssh { cause, .. } => core::error::Error::source(cause),
            Self::HandshakeClosed
            | Self::HandshakeTimeout { .. }
            | Self::ProtocolMismatch { .. } => None,
        }
    }
}

/// One handshaken connection to the daemon: an established, version-checked
/// framed transport plus the daemon's [`HelloAck`].
pub(crate) struct Connection {
    frames: Frames,
    ack: HelloAck,
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("ack", &self.ack)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Connects to `socket` and performs the version handshake (connect,
    /// send [`Hello`], read the reply), bounded by `timeout`. `dog_name`
    /// travels in the `Hello` so a refusing daemon knows which dog it refused.
    ///
    /// # Errors
    ///
    /// - [`ConnectError::Connect`]: the initial `connect(2)` call failed.
    /// - [`ConnectError::Wire`]: `Hello` failed to encode, or the reply failed to decode.
    /// - [`ConnectError::Io`]: a framed read or write failed after connect.
    /// - [`ConnectError::HandshakeClosed`]: the peer closed before a `HelloReply`.
    /// - [`ConnectError::HandshakeTimeout`]: no `HelloReply` arrived within `timeout`.
    /// - [`ConnectError::ProtocolMismatch`]: the daemon refused on protocol-version skew.
    pub(crate) async fn open(
        socket: &Path,
        timeout: Duration,
        dog_name: Option<&str>,
    ) -> Result<Self, ConnectError> {
        // bounds connect(2) together with the handshake, not just the handshake.
        tokio::time::timeout(timeout, Self::open_inner(socket, dog_name))
            .await
            .map_err(|_elapsed| ConnectError::HandshakeTimeout { after: timeout })?
    }

    async fn open_inner(socket: &Path, dog_name: Option<&str>) -> Result<Self, ConnectError> {
        let stream = transport::connect(socket)
            .await
            .map_err(|source| ConnectError::Connect {
                path: socket.to_path_buf(),
                source,
            })?;
        Self::open_over(stream, dog_name).await
    }

    /// Performs the version handshake (send [`Hello`], read the reply) over
    /// an already-established `stream`. `dog_name` travels in the `Hello`.
    ///
    /// Unbounded: the caller owns the timeout, as [`Self::open`] does.
    ///
    /// # Errors
    ///
    /// - [`ConnectError::Wire`]: `Hello` failed to encode, or the reply failed to decode.
    /// - [`ConnectError::Io`]: a framed read or write failed.
    /// - [`ConnectError::HandshakeClosed`]: the peer closed before a `HelloReply`.
    /// - [`ConnectError::ProtocolMismatch`]: the daemon refused on protocol-version skew.
    pub(crate) async fn open_over(
        stream: impl ByteStream + 'static,
        dog_name: Option<&str>,
    ) -> Result<Self, ConnectError> {
        let mut frames = Framed::new(Box::new(stream) as Box<dyn ByteStream>, codec());

        let hello = Hello {
            client_version: env!("CARGO_PKG_VERSION").to_string(),
            protocol: PROTOCOL_VERSION,
            dog_name: dog_name.map(str::to_owned),
        };
        let payload = encode_frame(&hello).map_err(ConnectError::Wire)?;
        frames.send(payload).await.map_err(ConnectError::Io)?;

        let frame = frames
            .next()
            .await
            .ok_or(ConnectError::HandshakeClosed)?
            .map_err(ConnectError::Io)?;
        let reply: HelloReply = decode_frame(&frame).map_err(ConnectError::Wire)?;
        // flattens every `RpcError` into `ProtocolMismatch`: `server.rs` is
        // the only producer and always sends that code
        let ack = reply.map_err(|err| ConnectError::ProtocolMismatch {
            client: PROTOCOL_VERSION,
            daemon_version: err.daemon_version,
            message: err.message,
        })?;

        Ok(Self { frames, ack })
    }

    /// Splits the connection into its raw framed transport and the
    /// handshake acknowledgement, for the actor task that takes ownership
    /// of the transport and needs both.
    pub(crate) fn into_parts(self) -> (Frames, HelloAck) {
        (self.frames, self.ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fake_daemon;
    use shep_core::protocol::{HelloAck, PROTOCOL_VERSION, RpcError, RpcErrorCode};
    use shep_core::transport::Listener;

    #[tokio::test]
    async fn open_sends_hello_and_returns_the_ack() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let ack = HelloAck {
            daemon_version: "9.9.9".into(),
            protocol: PROTOCOL_VERSION,
            pid: 4242,
            min_supported: None,
        };
        let served = fake_daemon(&path, Ok(ack.clone())).await;

        let conn = Connection::open(&path, HANDSHAKE_TIMEOUT, None)
            .await
            .unwrap();

        let (_frames, actual_ack) = conn.into_parts();
        assert_eq!(actual_ack, ack);
        let hello = served.await.unwrap();
        assert_eq!(
            hello.protocol, PROTOCOL_VERSION,
            "the client must announce the version it speaks"
        );
        assert_eq!(
            hello.client_version,
            env!("CARGO_PKG_VERSION"),
            "the client must identify its own version"
        );
        assert_eq!(
            hello.dog_name, None,
            "a client that is not a dog must not claim to be one"
        );
    }

    /// fails if a dog's name is dropped between [`Connection::open`]'s
    /// argument and the frame that goes out, which is all the daemon has
    /// to name the dog it refuses on a bad handshake.
    #[tokio::test]
    async fn a_dogs_hello_carries_the_name_it_was_registered_under() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let ack = HelloAck {
            daemon_version: "9.9.9".into(),
            protocol: PROTOCOL_VERSION,
            pid: 4242,
            min_supported: None,
        };
        let served = fake_daemon(&path, Ok(ack)).await;

        let _conn = Connection::open(&path, HANDSHAKE_TIMEOUT, Some("metrics"))
            .await
            .unwrap();

        let hello = served.await.unwrap();
        assert_eq!(hello.dog_name.as_deref(), Some("metrics"));
    }

    #[tokio::test]
    async fn a_protocol_refusal_becomes_its_own_error_variant() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let refusal = RpcError {
            code: RpcErrorCode::ProtocolMismatch,
            message: "daemon speaks protocol 2, client speaks 1".into(),
            daemon_version: None,
        };
        let _served = fake_daemon(&path, Err(refusal)).await;

        let err = Connection::open(&path, HANDSHAKE_TIMEOUT, None)
            .await
            .unwrap_err();

        let ConnectError::ProtocolMismatch {
            client, message, ..
        } = err
        else {
            panic!("a protocol refusal must not be flattened into a generic error, got {err:?}");
        };
        assert_eq!(
            client, PROTOCOL_VERSION,
            "`client` is our own version, not the daemon's"
        );
        assert!(
            message.contains("protocol 2"),
            "the daemon's own message must survive: {message}"
        );
    }

    #[tokio::test]
    async fn a_refusal_carries_the_daemon_version_past_the_flattening() {
        // only this side tests the RpcError -> ConnectError carry-through
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let refusal = RpcError {
            code: RpcErrorCode::ProtocolMismatch,
            message: "daemon speaks protocol 2, client speaks 1".into(),
            daemon_version: Some("0.1.16".into()),
        };
        let _served = fake_daemon(&path, Err(refusal)).await;

        let err = Connection::open(&path, HANDSHAKE_TIMEOUT, None)
            .await
            .unwrap_err();

        let ConnectError::ProtocolMismatch { daemon_version, .. } = err else {
            panic!("expected a protocol refusal, got {err:?}");
        };
        assert_eq!(daemon_version.as_deref(), Some("0.1.16"));
    }

    #[tokio::test]
    async fn an_old_daemons_refusal_still_connects_and_reports_no_version() {
        // `skip_serializing_if` means a daemon predating this field puts no
        // `daemon_version` on the wire at all; this must decode as `None`.
        // the byte-for-byte omission itself is pinned in shep-core; this
        // crate has no serde_json to assert it with.
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let refusal = RpcError {
            code: RpcErrorCode::ProtocolMismatch,
            message: "daemon speaks protocol 2, client speaks 1".into(),
            daemon_version: None,
        };
        let _served = fake_daemon(&path, Err(refusal)).await;

        let err = Connection::open(&path, HANDSHAKE_TIMEOUT, None)
            .await
            .unwrap_err();

        let ConnectError::ProtocolMismatch {
            daemon_version,
            message,
            ..
        } = err
        else {
            panic!("expected a protocol refusal, got {err:?}");
        };
        assert_eq!(daemon_version, None);
        assert!(
            message.contains("protocol 2"),
            "the daemon's own message must still survive: {message}"
        );
    }

    /// fails if the rendered refusal states the skew and stops without
    /// saying what to do about it: this string is the whole of what a
    /// refused dog's log gets.
    #[test]
    fn a_rendered_refusal_says_what_to_do_about_it() {
        let rendered = ConnectError::ProtocolMismatch {
            client: 1,
            daemon_version: Some("0.1.27".into()),
            message: "daemon speaks protocol 2, client sent 1".into(),
        }
        .to_string();

        assert!(
            rendered.contains("daemon speaks protocol 2, client sent 1"),
            "the daemon's own sentence still leads: {rendered}"
        );
        assert!(
            rendered.contains("shep 0.1.27"),
            "the remedy has to name a version somebody can install: {rendered}"
        );
        assert!(
            rendered.contains("rebuild or reinstall this program"),
            "one direction of the remedy: {rendered}"
        );
        assert!(
            rendered.contains("shep daemon reload"),
            "the other direction, because this type cannot tell which build \
             is the older one: {rendered}"
        );
        assert_eq!(rendered.lines().count(), 1, "one line: {rendered}");
    }

    /// fails if a daemon too old to name its version renders a remedy with a
    /// hole in it: `rebuild against shep` with nothing after `shep`, or a
    /// literal `None`.
    #[test]
    fn a_refusal_without_a_version_still_names_something_to_build_against() {
        let rendered = ConnectError::ProtocolMismatch {
            client: 2,
            daemon_version: None,
            message: "daemon speaks protocol 1, client sent 2".into(),
        }
        .to_string();

        assert!(
            rendered.contains("against the running shep and restart it"),
            "the remedy must still point somewhere: {rendered}"
        );
        assert!(
            !rendered.contains("None"),
            "an absent version must never reach the reader: {rendered}"
        );
    }

    /// fails if a daemon that accepts and immediately closes is read as
    /// anything but unreachable, or as a silent success. Asserts the
    /// outcome bucket, not one variant: macOS reports `HandshakeClosed`,
    /// Linux reports `Io`, and downstream folds both the same way.
    #[tokio::test]
    async fn a_daemon_that_closes_without_answering_is_not_a_silent_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let mut listener = Listener::bind(&path).unwrap();
        tokio::spawn(async move {
            let stream = listener.accept().await.unwrap();
            drop(stream);
        });

        let err = Connection::open(&path, HANDSHAKE_TIMEOUT, None)
            .await
            .expect_err("a peer that closed without a HelloReply is not a connection");

        assert!(
            matches!(err, ConnectError::HandshakeClosed | ConnectError::Io(_)),
            "a peer that closed mid-handshake must report as unreachable, got {err:?}"
        );
    }

    #[tokio::test]
    async fn connecting_to_a_missing_socket_names_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        let ConnectError::Connect { path: reported, .. } =
            Connection::open(&path, HANDSHAKE_TIMEOUT, None)
                .await
                .unwrap_err()
        else {
            panic!("a missing socket must report which path failed");
        };
        assert_eq!(reported, path);
    }

    /// 150ms stands in for the real 5s [`HANDSHAKE_TIMEOUT`]; the mechanism is the same.
    #[tokio::test]
    async fn a_socket_bound_but_never_accepted_from_times_out_rather_than_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let path = crate::testing::control_address(dir.path());
        // unix completes connect() into the backlog; Windows completes it
        // against an unaccepted pipe instance. Either way nothing answers.
        let _listener = Listener::bind(&path).unwrap();

        let err = Connection::open(&path, Duration::from_millis(150), None)
            .await
            .unwrap_err();

        let ConnectError::HandshakeTimeout { after } = err else {
            panic!("a backlogged connect must time out, not hang or read as success; got {err:?}");
        };
        assert_eq!(after, Duration::from_millis(150));
    }

    /// Spawns the far end of an in-memory stream that reads one [`Hello`]
    /// and answers it with `reply`, then hands the [`Hello`] back.
    fn serve_hello(
        far: tokio::io::DuplexStream,
        reply: HelloReply,
    ) -> tokio::task::JoinHandle<Hello> {
        tokio::spawn(async move {
            let mut frames = Framed::new(far, codec());
            let first = frames.next().await.unwrap().unwrap();
            let hello: Hello = decode_frame(&first).unwrap();
            frames.send(encode_frame(&reply).unwrap()).await.unwrap();
            hello
        })
    }

    #[tokio::test]
    async fn open_over_completes_the_handshake_on_an_in_memory_stream() {
        let (near, far) = tokio::io::duplex(1024);
        let ack = HelloAck {
            daemon_version: "9.9.9".into(),
            protocol: PROTOCOL_VERSION,
            pid: 4242,
            min_supported: None,
        };
        let served = serve_hello(far, Ok(ack.clone()));

        let conn = Connection::open_over(near, None).await.unwrap();

        let (_frames, actual_ack) = conn.into_parts();
        assert_eq!(actual_ack, ack);
        let hello = served.await.unwrap();
        assert_eq!(
            hello.protocol, PROTOCOL_VERSION,
            "the client must announce the version it speaks"
        );
        assert_eq!(
            hello.client_version,
            env!("CARGO_PKG_VERSION"),
            "the client must identify its own version"
        );
        assert_eq!(
            hello.dog_name, None,
            "a client that is not a dog must not claim to be one"
        );
    }

    #[tokio::test]
    async fn open_over_carries_a_dogs_name_in_its_hello() {
        let (near, far) = tokio::io::duplex(1024);
        let ack = HelloAck {
            daemon_version: "9.9.9".into(),
            protocol: PROTOCOL_VERSION,
            pid: 4242,
            min_supported: None,
        };
        let served = serve_hello(far, Ok(ack));

        let _conn = Connection::open_over(near, Some("metrics")).await.unwrap();

        let hello = served.await.unwrap();
        assert_eq!(hello.dog_name.as_deref(), Some("metrics"));
    }

    #[tokio::test]
    async fn open_over_reports_a_stream_closed_before_the_ack_as_handshake_closed() {
        let (near, far) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let mut frames = Framed::new(far, codec());
            let _hello = frames.next().await;
            // dropped after reading the Hello, so the near half sees a clean EOF
        });

        let err = Connection::open_over(near, None).await.unwrap_err();

        assert!(
            matches!(err, ConnectError::HandshakeClosed),
            "an in-memory EOF carries no OS error to fold into `Io`, got {err:?}"
        );
    }

    #[tokio::test]
    async fn open_over_reports_a_protocol_refusal_as_protocol_mismatch() {
        let (near, far) = tokio::io::duplex(1024);
        let refusal = RpcError {
            code: RpcErrorCode::ProtocolMismatch,
            message: "daemon speaks protocol 2, client speaks 1".into(),
            daemon_version: Some("0.1.16".into()),
        };
        let _served = serve_hello(far, Err(refusal));

        let err = Connection::open_over(near, None).await.unwrap_err();

        let ConnectError::ProtocolMismatch {
            client,
            daemon_version,
            message,
        } = err
        else {
            panic!("a protocol refusal must not be flattened into a generic error, got {err:?}");
        };
        assert_eq!(client, PROTOCOL_VERSION);
        assert_eq!(daemon_version.as_deref(), Some("0.1.16"));
        assert!(
            message.contains("protocol 2"),
            "the daemon's own message must survive: {message}"
        );
    }

    /// A refusal is the one stage an upgrade fixes; every other stage is
    /// nothing answering.
    #[test]
    fn a_refusal_exits_protocol_mismatch_and_every_other_stage_unreachable() {
        let refused = ConnectError::ProtocolMismatch {
            client: 1,
            daemon_version: None,
            message: "too old".to_owned(),
        };
        assert_eq!(refused.exit_code(), shep_core::exit::PROTOCOL_MISMATCH);
        for unreachable in [
            ConnectError::HandshakeClosed,
            ConnectError::Io(std::io::Error::other("reset")),
        ] {
            assert_eq!(unreachable.exit_code(), shep_core::exit::DAEMON_UNREACHABLE);
        }
    }

    /// Every ssh failure is nothing usable answering, whatever its cause; a
    /// refusal through ssh is still the refusal, and keeps its own code.
    #[test]
    fn ssh_failures_exit_daemon_unreachable_and_a_refusal_keeps_protocol_mismatch() {
        let causes = [
            SshCause::Spawn {
                program: PathBuf::from("ssh"),
                source: std::io::Error::other("no such file"),
            },
            SshCause::Transport {
                stderr: "Connection refused".to_owned(),
            },
            SshCause::CommandNotFound,
            SshCause::NoShepherd,
            SshCause::RemoteTooOld,
            SshCause::LoginOutput(Box::new(ConnectError::HandshakeClosed)),
        ];
        for cause in causes {
            let err = ConnectError::Ssh {
                destination: "build-01".to_owned(),
                cause,
            };
            assert_eq!(
                err.exit_code(),
                shep_core::exit::DAEMON_UNREACHABLE,
                "{err}"
            );
        }

        let refused = ConnectError::ProtocolMismatch {
            client: 1,
            daemon_version: None,
            message: "too old".to_owned(),
        };
        assert_eq!(refused.exit_code(), shep_core::exit::PROTOCOL_MISMATCH);
    }

    /// `Display` names the host, and `source` reaches the cause underneath
    /// without repeating the message `Display` already printed.
    #[test]
    fn an_ssh_failure_names_its_host_and_exposes_the_cause_beneath_it() {
        let spawn = ConnectError::Ssh {
            destination: "build-01".to_owned(),
            cause: SshCause::Spawn {
                program: PathBuf::from("/opt/ssh"),
                source: std::io::Error::other("denied"),
            },
        };
        assert!(spawn.to_string().contains("`/opt/ssh`"), "{spawn}");
        assert_eq!(
            core::error::Error::source(&spawn).unwrap().to_string(),
            "denied"
        );

        let wrapped = ConnectError::Ssh {
            destination: "build-01".to_owned(),
            cause: SshCause::LoginOutput(Box::new(ConnectError::HandshakeClosed)),
        };
        assert!(wrapped.to_string().contains("`build-01`"), "{wrapped}");
        assert_eq!(
            core::error::Error::source(&wrapped).unwrap().to_string(),
            ConnectError::HandshakeClosed.to_string()
        );

        let transport = ConnectError::Ssh {
            destination: "build-01".to_owned(),
            cause: SshCause::NoShepherd,
        };
        assert!(core::error::Error::source(&transport).is_none());
    }

    #[test]
    fn a_refused_destination_is_a_usage_error() {
        let err = ConnectError::Destination(DestinationError::Empty);
        assert_eq!(err.exit_code(), shep_core::exit::USAGE);
        assert!(core::error::Error::source(&err).is_some());
    }
}
