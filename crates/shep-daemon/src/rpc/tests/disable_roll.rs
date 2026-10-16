//! `Disable` and `Enable` reach the roll file without a graceful shutdown.

use super::*;

fn by_name(name: &str) -> SelectorSpec {
    SelectorSpec::Name(name.to_string())
}

/// Fails if disabling an app that is already `Stopped` stops waking the roll
/// writer: no process moves, so no bus event says anything, and a `SIGKILL`
/// afterwards would bring the app back enabled.
#[tokio::test(start_paused = true)]
async fn disabling_a_stopped_app_reaches_the_roll_before_a_hard_stop_can_lose_it() {
    let h = harness(vec![ProcScript::never_exits()]);
    for (id, request) in [
        (
            1,
            Request::Start {
                apps: vec![AppConfig::minimal("api", "./srv")],
            },
        ),
        (
            2,
            Request::Stop {
                selector: by_name("api"),
            },
        ),
    ] {
        let reply = reply_of(dispatch(envelope(id, request), &h.ctx).await);
        assert!(reply.result.is_ok(), "{:?}", reply.result);
    }
    h.ctx.save_roll_now().await.unwrap();
    assert!(!crate::snapshot::read(&h.ctx.snapshot_path).unwrap().apps[0].disabled);
    let writer = crate::snapshot::spawn_snapshot_writer(
        h.ctx.snapshot_path.clone(),
        h.ctx.supervisor.clone(),
        h.ctx.registry.clone(),
        h.ctx.events.subscribe(),
    );
    let settle = std::time::Duration::from_millis(crate::snapshot::SNAPSHOT_DEBOUNCE_MS * 2);
    tokio::time::sleep(settle).await;

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Disable {
                    selector: by_name("api"),
                },
            ),
            &h.ctx,
        )
        .await,
    );
    assert!(reply.result.is_ok(), "{:?}", reply.result);
    tokio::time::sleep(settle).await;
    writer.stop().await;

    let rolled = crate::snapshot::read(&h.ctx.snapshot_path).unwrap();
    assert!(
        rolled.apps[0].disabled,
        "the roll still says api is enabled"
    );
}
