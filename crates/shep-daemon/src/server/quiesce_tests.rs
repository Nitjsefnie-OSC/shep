//! A handover freezes a served dog link on a frame boundary: the stream a
//! successor inherits starts a frame, never the middle of one.

use core::time::Duration;

use super::resume_tests::{ping, serve};
use super::testing::start_dog;
use crate::fake::ProcScript;
use crate::testing::harness;
use shep_core::protocol::{Envelope, Request, Response, ServerFrame, encode_frame};
use tokio::io::AsyncWriteExt as _;

const PARK: Duration = Duration::from_secs(5);

/// A `Ping` request as it goes on the wire, length prefix included.
fn ping_on_the_wire(id: u64) -> Vec<u8> {
    let payload = encode_frame(&Envelope {
        id,
        deadline_ms: None,
        body: Request::Ping,
    })
    .unwrap();
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(&payload);
    bytes
}

#[tokio::test]
async fn an_idle_link_freezes_at_once_and_serves_on_after_a_thaw() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_dog(&h.ctx, "metrics").await;
    let mut served = serve(&h.ctx, "metrics", Some(vec![])).await;

    let carried = tokio::time::timeout(PARK, served.table.freeze(PARK))
        .await
        .expect("an idle link parks without waiting out the timeout");
    assert_eq!(carried.len(), 1);

    served.table.thaw();
    let frame = ping(&mut served.client, 3).await;
    assert!(
        matches!(&frame, ServerFrame::Reply(reply) if reply.result == Ok(Response::Pong)),
        "{frame:?}"
    );
}

#[tokio::test]
async fn a_freeze_waits_for_a_request_the_dog_is_part_way_through_sending() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_dog(&h.ctx, "metrics").await;
    let mut served = serve(&h.ctx, "metrics", Some(vec![])).await;
    let wire = ping_on_the_wire(9);
    let (head, tail) = wire.split_at(6);
    served
        .client
        .frames
        .get_mut()
        .write_all(head)
        .await
        .unwrap();
    served.client.frames.get_mut().flush().await.unwrap();
    // The reader has taken the header and the start of the payload.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let table = served.table.clone();
    let freezing = tokio::spawn(async move { table.freeze(PARK).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !freezing.is_finished(),
        "a link mid-frame must not be copied"
    );

    served
        .client
        .frames
        .get_mut()
        .write_all(tail)
        .await
        .unwrap();
    served.client.frames.get_mut().flush().await.unwrap();
    let carried = tokio::time::timeout(PARK, freezing)
        .await
        .expect("the freeze ends once the frame does")
        .unwrap();
    assert_eq!(carried.len(), 1, "the link parks on the boundary after it");

    // The reply to the request that finished is held with the writer, not lost.
    served.table.thaw();
    let frame = served.client.recv().await;
    assert!(
        matches!(&frame, ServerFrame::Reply(reply) if reply.id == 9 && reply.result == Ok(Response::Pong)),
        "{frame:?}"
    );
}

#[tokio::test]
async fn a_link_stuck_mid_frame_is_left_out_of_the_copy() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_dog(&h.ctx, "metrics").await;
    let mut served = serve(&h.ctx, "metrics", Some(vec![])).await;
    let wire = ping_on_the_wire(9);
    served
        .client
        .frames
        .get_mut()
        .write_all(&wire[..6])
        .await
        .unwrap();
    served.client.frames.get_mut().flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let carried = served.table.freeze(Duration::from_millis(300)).await;

    assert!(carried.is_empty());
}
