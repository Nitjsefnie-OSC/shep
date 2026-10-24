//! When `EnableDog` records a dog's grants: before the dog can be served, and
//! only for a dog.

use super::*;
use shep_core::permissions::{Grants, Permission};
use std::time::Duration;

fn adopted_with(grants: Grants) -> Request {
    Request::EnableDog {
        name: "otel".to_string(),
        source: DogSource::Adopted {
            grants: Some(grants),
            path: "/usr/local/bin/shep-otel".to_string(),
            channel: false,
        },
    }
}

/// The supervisor starts a dog it was queued even when the request's
/// deadline drops the dispatch first, so the grants cannot wait for the
/// start's answer: until they are recorded the dog is undeclared, with
/// full access.
#[tokio::test(start_paused = true)]
async fn a_dispatch_cut_short_after_queueing_the_start_still_records_the_grants() {
    let h = harness(vec![ProcScript::never_exits()]);
    let grants = Grants::new([Permission::FlockRead]);

    let cut = tokio::time::timeout(
        Duration::ZERO,
        dispatch(envelope(1, adopted_with(grants.clone())), &h.ctx),
    )
    .await;

    assert!(cut.is_err(), "the dispatch was meant to be cut short");
    assert_eq!(h.ctx.known_dogs.grants("otel"), Some(grants));
}

/// A sheep holding the name is refused, and that is not a dog to remember:
/// the name stays unknown and keeps no grants.
#[tokio::test(start_paused = true)]
async fn an_enable_refused_for_a_sheeps_name_remembers_no_dog() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;
    let request = Request::EnableDog {
        name: "web".to_string(),
        source: DogSource::Adopted {
            grants: Some(Grants::default()),
            path: "/usr/local/bin/shep-web".to_string(),
            channel: false,
        },
    };

    let refused = reply_of(dispatch(envelope(2, request), &h.ctx).await).result;

    let Err(err) = refused else {
        panic!("a sheep's name must be refused, got {refused:?}")
    };
    assert_eq!(err.code, RpcErrorCode::InvalidConfig, "{err:?}");
    assert!(!h.ctx.known_dogs.contains("web"));
}

/// A name that was already a dog keeps what it was granted when a later
/// enable of it is refused.
#[tokio::test(start_paused = true)]
async fn a_refused_enable_leaves_the_grants_the_name_already_held() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;
    let before = Grants::new([Permission::LogsRead]);
    h.ctx.known_dogs.insert("web", Some(before.clone()));
    let request = Request::EnableDog {
        name: "web".to_string(),
        source: DogSource::Adopted {
            grants: Some(Grants::default()),
            path: "/usr/local/bin/shep-web".to_string(),
            channel: false,
        },
    };

    let _ = dispatch(envelope(2, request), &h.ctx).await;

    assert_eq!(h.ctx.known_dogs.grants("web"), Some(before));
}
