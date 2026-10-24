//! A dog's carried subscription meets the successor's grants, which may be
//! narrower than the ones it subscribed under.

use shep_core::permissions::{Grants, Permission};
use shep_core::protocol::{BusEvent, ServerFrame};

use super::resume_tests::{ping, serve};
use super::testing::*;
use crate::fake::ProcScript;
use crate::testing::harness;

#[tokio::test]
async fn a_carried_subscription_outside_the_grants_is_dropped_not_re_armed() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_dog(&h.ctx, "metrics").await;
    h.ctx
        .known_dogs
        .insert("metrics", Some(Grants::new([Permission::FlockRead])));
    let mut served = serve(&h.ctx, "metrics", Some(vec!["*".to_string()])).await;

    // No drop notice ahead of the reply: nothing was re-armed.
    let frame = ping(&mut served.client, 4).await;

    assert!(
        matches!(&frame, ServerFrame::Reply(reply) if reply.id == 4),
        "{frame:?}"
    );
    h.ctx
        .events
        .send(
            BusEvent::LogOut {
                id: 0,
                line: "after the exec".to_string(),
            }
            .into(),
        )
        .unwrap();
    let after = ping(&mut served.client, 5).await;
    assert!(
        matches!(&after, ServerFrame::Reply(reply) if reply.id == 5),
        "an event reached a dog with no grant for it: {after:?}"
    );
    assert_eq!(
        served.table.snapshot()[0].topics,
        Some(vec![]),
        "the next handover must not carry the refused subscription"
    );
}

#[tokio::test]
async fn a_carried_subscription_inside_the_grants_is_re_armed() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_dog(&h.ctx, "metrics").await;
    h.ctx.known_dogs.insert(
        "metrics",
        Some(Grants::new([Permission::Events("log.".to_string())])),
    );
    let mut served = serve(&h.ctx, "metrics", Some(vec!["log.out".to_string()])).await;

    let notice: BusEvent = served.client.recv().await;

    assert!(
        matches!(notice, BusEvent::Dropped { count } if count >= 1),
        "{notice:?}"
    );
}
