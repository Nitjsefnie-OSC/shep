//! A subscription a dog already holds meets a grant narrowed while it runs.

use shep_core::permissions::{Grants, Permission};
use shep_core::protocol::{BusEvent, ServerFrame};

use super::resume_tests::{ping, serve};
use super::testing::*;
use crate::fake::ProcScript;
use crate::testing::harness;

fn log_line(line: &str) -> shep_core::protocol::BusEvent {
    BusEvent::LogOut {
        id: 0,
        line: line.to_string(),
    }
}

#[tokio::test]
async fn a_revoke_stops_the_events_of_a_subscription_already_held() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_dog(&h.ctx, "metrics").await;
    h.ctx.known_dogs.insert(
        "metrics",
        Some(Grants::new([Permission::Events("log.".to_string())])),
    );
    let mut served = serve(&h.ctx, "metrics", Some(vec!["log.out".to_string()])).await;
    let _notice: BusEvent = served.client.recv().await;
    h.ctx.events.send(log_line("while granted").into()).unwrap();
    let heard: BusEvent = served.client.recv().await;
    assert!(matches!(heard, BusEvent::LogOut { .. }), "{heard:?}");

    h.ctx
        .known_dogs
        .insert("metrics", Some(Grants::new([Permission::FlockRead])));
    h.ctx
        .events
        .send(log_line("after the revoke").into())
        .unwrap();
    let after = ping(&mut served.client, 7).await;

    assert!(
        matches!(&after, ServerFrame::Reply(reply) if reply.id == 7),
        "an event reached a dog whose grant was revoked: {after:?}"
    );
}
