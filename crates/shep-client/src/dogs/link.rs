//! How a dog's first connection is made: over the one the shepherd spawned
//! it holding, or through the socket when there is none.

use std::path::Path;

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
/// See [`ReconnectingClient::connect_as`].
pub(super) async fn connect(
    identity: &DogIdentity,
    socket: &Path,
) -> Result<ReconnectingClient, ConnectError> {
    #[cfg(unix)]
    return connect_via(inherited(), identity, socket).await;
    #[cfg(not(unix))]
    return ReconnectingClient::connect_as(socket, identity).await;
}

/// The connection the shepherd spawned this process holding, if it did and
/// it can be used. A dog started by hand has none.
#[cfg(unix)]
fn inherited() -> Option<tokio::net::UnixStream> {
    let std_stream = shep_channel::take_inherited_socket(shep_core::dogs::DOG_LINK_FD_VAR)
        .ok()
        .flatten()?;
    std_stream.set_nonblocking(true).ok()?;
    tokio::net::UnixStream::from_std(std_stream).ok()
}

/// As [`connect`], with the inherited connection passed in.
#[cfg(unix)]
pub(super) async fn connect_via(
    link: Option<tokio::net::UnixStream>,
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
        Envelope, Hello, HelloReply, Reply, Request, Response, codec, decode_frame,
        encode_frame,
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
