//! What the daemon does with a `Hello` naming a protocol version it does
//! not share, and with a request it does not recognise.

use super::*;

/// Connects to `fixture`'s socket by hand, sends a `Hello` naming `protocol`
/// and hands back the still-open frame stream beside the daemon's reply.
///
/// [`Fixture::connect`] always sends a matching protocol and keeps no handle,
/// so a test that needs the socket after the handshake goes through here.
async fn hello_with_protocol(
    fixture: &Fixture,
    protocol: u32,
) -> (Framed<ClientStream, LengthDelimitedCodec>, HelloReply) {
    let stream = transport::connect(&fixture.paths.socket).await.unwrap();
    let mut frames = Framed::new(stream, codec());
    frames
        .send(
            encode_frame(&Hello {
                client_version: "9.9.9".to_string(),
                protocol,
                dog_name: None,
            })
            .unwrap(),
        )
        .await
        .unwrap();

    let frame = tokio::time::timeout(RECV_TIMEOUT, frames.next())
        .await
        .expect("timed out waiting for the handshake reply")
        .expect("connection closed before replying")
        .unwrap();
    let ack: HelloReply = decode_frame(&frame).unwrap();
    (frames, ack)
}

/// The handshake alone: drops the connection and returns the reply.
async fn handshake_with_protocol(fixture: &Fixture, protocol: u32) -> Result<HelloAck, RpcError> {
    let (_frames, ack) = hello_with_protocol(fixture, protocol).await;
    ack
}

#[tokio::test]
async fn a_peer_at_the_floor_is_accepted() {
    let fixture = Fixture::boot(tempfile::tempdir().unwrap(), false).await;

    let ack = handshake_with_protocol(&fixture, MIN_SUPPORTED)
        .await
        .expect("at the floor");
    assert_eq!(ack.protocol, PROTOCOL_VERSION);
    assert_eq!(ack.min_supported, Some(MIN_SUPPORTED));

    fixture.shutdown().await;
}

#[tokio::test]
async fn a_peer_below_the_floor_is_refused_by_name() {
    let fixture = Fixture::boot(tempfile::tempdir().unwrap(), false).await;

    let err = handshake_with_protocol(&fixture, MIN_SUPPORTED - 1)
        .await
        .expect_err("below the floor");
    assert_eq!(err.code, RpcErrorCode::ProtocolMismatch);

    fixture.shutdown().await;
}

/// A dog rebuilt against a newer shep-client than the running shepherd.
/// Refusing it bought nothing: anything it asks for that does not exist
/// is refused per request by `Request::Unrecognized`.
#[tokio::test]
async fn a_peer_above_the_daemons_own_version_is_accepted() {
    let fixture = Fixture::boot(tempfile::tempdir().unwrap(), false).await;

    let ack = handshake_with_protocol(&fixture, PROTOCOL_VERSION + 1)
        .await
        .expect("a newer peer connects");
    assert_eq!(ack.protocol, PROTOCOL_VERSION);

    fixture.shutdown().await;
}

#[tokio::test]
async fn a_newer_peer_keeps_the_connection_open_over_the_real_socket() {
    let fixture = Fixture::boot(tempfile::tempdir().unwrap(), false).await;

    let (mut frames, ack) = hello_with_protocol(&fixture, PROTOCOL_VERSION + 1).await;
    ack.expect("a peer newer than the daemon must be accepted, not refused as skew");

    // A live connection accepts a request rather than staying silent.
    let frame = encode_frame(&Envelope {
        id: 1,
        deadline_ms: None,
        body: Request::Ping,
    })
    .unwrap();
    frames.send(frame).await.unwrap();
    let reply = tokio::time::timeout(RECV_TIMEOUT, frames.next())
        .await
        .expect("timed out waiting for the ping reply")
        .expect("connection closed before replying")
        .unwrap();
    let frame: ServerFrame = decode_frame(&reply).unwrap();
    let ServerFrame::Reply(reply) = frame else {
        panic!("expected a reply frame")
    };
    assert_eq!(reply.result, Ok(Response::Pong));

    fixture.shutdown().await;
}

/// The whole point of the catch-all. An unknown request must be refused BY
/// ID and leave the connection usable, because the alternative is the
/// dropped connection that made every additive change a version bump.
#[tokio::test]
async fn an_unrecognized_request_is_refused_and_the_connection_survives() {
    let fixture = Fixture::boot(tempfile::tempdir().unwrap(), false).await;
    let mut conn = fixture.connect().await;

    let refusal = conn
        .request_raw(serde_json::json!({"kind": "from_the_future"}))
        .await
        .expect("an unrecognized request is refused, not disconnected");
    let Err(err) = refusal.result else {
        panic!("an unknown request must be refused");
    };
    assert_eq!(err.code, RpcErrorCode::Unsupported);
    assert_eq!(
        err.daemon_version, None,
        "daemon_version is reserved for a ProtocolMismatch refusal; this \
         client already has it from HelloAck"
    );

    let pong = conn.request(Request::Ping).await;
    assert!(
        pong.result.is_ok(),
        "the connection must still serve after refusing one request"
    );

    fixture.shutdown().await;
}
