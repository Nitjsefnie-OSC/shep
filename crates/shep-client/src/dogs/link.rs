//! How a dog's first connection is made: over the one the shepherd spawned
//! it holding, or through the socket when there is none.

use std::path::Path;

use shep_core::transport::ClientStream;

use crate::connection::HANDSHAKE_TIMEOUT;
use crate::{ConnectError, ReconnectingClient};

use super::DogIdentity;

/// Connects `identity` to the shepherd at `socket`.
///
/// Over the inherited connection when the shepherd handed one down, which
/// is served from spawn and so never waits on the listener. The socket
/// otherwise, and when that connection does not complete a handshake.
///
/// # Errors
///
/// - [`ConnectError::Io`]: `$SHEP_DOG_FD` (`$SHEP_DOG_PIPE` on Windows) is
///   set and names no usable connection. The shepherd said this dog has one,
///   so reaching it on the socket as the operator would hide the fault.
/// - Otherwise as [`ReconnectingClient::connect_as`].
pub(super) async fn connect(
    identity: &DogIdentity,
    socket: &Path,
) -> Result<ReconnectingClient, ConnectError> {
    #[cfg(unix)]
    let link = inherited()?;
    #[cfg(windows)]
    let link = inherited().await?;
    connect_via(link, identity, socket).await
}

/// Whether this process has opened its pipe already. A dog has one
/// connection from its shepherd: the shepherd serves the first open and
/// drops the pipe, so a second would find it busy for as long as the first
/// lives.
#[cfg(windows)]
static PIPE_TAKEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// The connection the shepherd spawned this process holding, opened by the
/// name in `$SHEP_DOG_PIPE`. `None` for a dog started by hand, and for one
/// that already took it.
#[cfg(windows)]
async fn inherited() -> Result<Option<ClientStream>, ConnectError> {
    use core::sync::atomic::Ordering;

    let var = shep_core::dogs::DOG_LINK_PIPE_VAR;
    let Some(pipe) = std::env::var_os(var) else {
        return Ok(None);
    };
    if PIPE_TAKEN.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let unusable = |reason: &dyn core::fmt::Display| {
        ConnectError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{var}: {reason}"),
        ))
    };
    // Bounded: a pipe the shepherd is not accepting on answers busy, and the
    // transport retries a busy pipe for as long as the caller lets it.
    let opened = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        shep_core::transport::connect(Path::new(&pipe)),
    )
    .await
    .map_err(|_elapsed| unusable(&"the pipe stayed busy"))?
    .map_err(|error| unusable(&error))?;
    PIPE_TAKEN.store(true, Ordering::SeqCst);
    Ok(Some(opened))
}

/// The connection the shepherd spawned this process holding. `None` for a
/// dog started by hand, and for one that already took it.
#[cfg(unix)]
fn inherited() -> Result<Option<ClientStream>, ConnectError> {
    adopt(shep_channel::take_inherited_socket(
        shep_core::dogs::DOG_LINK_FD_VAR,
    ))
}

/// Makes what `take_inherited_socket` found into a connection this runtime
/// can use.
#[cfg(unix)]
fn adopt(
    taken: Result<Option<std::os::unix::net::UnixStream>, shep_channel::ChannelError>,
) -> Result<Option<ClientStream>, ConnectError> {
    let unusable = |reason: &dyn core::fmt::Display| {
        ConnectError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{}: {reason}", shep_core::dogs::DOG_LINK_FD_VAR),
        ))
    };
    match taken {
        Ok(None) | Err(shep_channel::ChannelError::AlreadyTaken) => Ok(None),
        Err(error) => Err(unusable(&error)),
        Ok(Some(stream)) => {
            stream.set_nonblocking(true).map_err(ConnectError::Io)?;
            tokio::net::UnixStream::from_std(stream)
                .map(Some)
                .map_err(ConnectError::Io)
        }
    }
}

/// As [`connect`], with the inherited connection passed in.
pub(super) async fn connect_via(
    link: Option<ClientStream>,
    identity: &DogIdentity,
    socket: &Path,
) -> Result<ReconnectingClient, ConnectError> {
    if let Some(link) = link {
        let over = ReconnectingClient::connect_over_link(
            link,
            socket,
            HANDSHAKE_TIMEOUT,
            identity.handshake(),
        )
        .await;
        if let Ok(client) = over {
            return Ok(client);
        }
    }
    ReconnectingClient::connect_as(socket, identity).await
}

#[cfg(all(test, unix))]
mod tests {
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use shep_core::protocol::{
        Envelope, Hello, HelloReply, Reply, Request, Response, codec, decode_frame, encode_frame,
    };
    use tokio::net::UnixStream;
    use tokio::task::JoinHandle;
    use tokio_util::codec::Framed;

    use super::*;
    use crate::testing::{control_address, fake_daemon, sample_ack};

    /// The shepherd's end of a link: handshakes, then answers each request
    /// with a `DogSection` until the dog hangs up.
    /// Returns the `Hello` and every request seen.
    fn serve_link(shepherd: UnixStream) -> JoinHandle<(Hello, Vec<Request>)> {
        tokio::spawn(async move {
            let mut frames = Framed::new(shepherd, codec());
            let first = frames.next().await.unwrap().unwrap();
            let hello: Hello = decode_frame(&first).unwrap();
            let reply: HelloReply = Ok(sample_ack());
            frames.send(encode_frame(&reply).unwrap()).await.unwrap();
            let mut seen = Vec::new();
            while let Some(Ok(frame)) = frames.next().await {
                let envelope: Envelope = decode_frame(&frame).unwrap();
                seen.push(envelope.body.clone());
                let reply = Reply {
                    id: envelope.id,
                    result: Ok(Response::DogSection {
                        toml: "from = \"link\"\n".to_owned().into(),
                    }),
                };
                frames.send(encode_frame(&reply).unwrap()).await.unwrap();
            }
            (hello, seen)
        })
    }

    #[tokio::test]
    async fn a_variable_that_names_no_connection_is_an_error_and_not_a_fallback() {
        let taken = Err(shep_channel::ChannelError::Unusable(
            "SHEP_DOG_FD=fido".to_owned(),
        ));

        let err = adopt(taken).unwrap_err();

        assert!(matches!(err, ConnectError::Io(_)), "{err:?}");
        assert!(err.to_string().contains("SHEP_DOG_FD"), "{err}");
    }

    #[tokio::test]
    async fn no_variable_and_a_link_already_taken_both_mean_the_socket() {
        assert!(adopt(Ok(None)).unwrap().is_none());
        assert!(
            adopt(Err(shep_channel::ChannelError::AlreadyTaken))
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_taken_link_becomes_a_usable_stream() {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(adopt(Ok(Some(ours))).unwrap().is_some());
    }

    #[tokio::test]
    async fn a_dog_with_a_link_handshakes_on_it_and_never_dials_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = control_address(dir.path());
        let (dog_end, shepherd_end) = UnixStream::pair().unwrap();
        let served = serve_link(shepherd_end);

        let client = connect_via(Some(dog_end), &DogIdentity::named("bark"), &socket)
            .await
            .unwrap();
        let section = client.dog_config("bark").await.unwrap();
        drop(client);

        assert_eq!(section.as_str(), "from = \"link\"\n");
        let (hello, seen) = served.await.unwrap();
        assert_eq!(hello.dog_name.as_deref(), Some("bark"));
        assert_eq!(seen.len(), 1);
        assert!(!socket.exists(), "nothing was ever bound at the socket");
    }

    #[tokio::test]
    async fn a_dog_without_a_link_connects_on_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = control_address(dir.path());
        let hello = fake_daemon(&socket, Ok(sample_ack())).await;

        let client = connect_via(None, &DogIdentity::named("bark"), &socket)
            .await
            .unwrap();

        assert_eq!(hello.await.unwrap().dog_name.as_deref(), Some("bark"));
        assert_eq!(client.socket(), socket);
    }

    #[tokio::test]
    async fn a_link_that_never_handshakes_falls_back_to_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = control_address(dir.path());
        let (dog_end, shepherd_end) = UnixStream::pair().unwrap();
        drop(shepherd_end);
        let hello = fake_daemon(&socket, Ok(sample_ack())).await;

        connect_via(Some(dog_end), &DogIdentity::named("bark"), &socket)
            .await
            .unwrap();

        assert_eq!(hello.await.unwrap().dog_name.as_deref(), Some("bark"));
    }

    #[tokio::test]
    async fn a_lost_link_is_replaced_by_a_connection_on_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let socket = control_address(dir.path());
        let (dog_end, shepherd_end) = UnixStream::pair().unwrap();
        let served = serve_link(shepherd_end);
        let client = connect_via(Some(dog_end), &DogIdentity::named("bark"), &socket)
            .await
            .unwrap();
        let hello = fake_daemon(&socket, Ok(sample_ack())).await;

        served.abort();
        let _ = served.await;

        let hello = tokio::time::timeout(Duration::from_secs(5), hello)
            .await
            .expect("the dog dialled the socket once its link was lost")
            .unwrap();
        assert_eq!(hello.dog_name.as_deref(), Some("bark"));
        drop(client);
    }
}
