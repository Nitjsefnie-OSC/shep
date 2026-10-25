//! A restart against a disabled app, by who asked: an operator is refused
//! with `RpcErrorCode::Disabled`, the shepherd's own restarts are dropped, and
//! either way the app stays stopped.

use core::time::Duration;

use shep_core::selector::ProcessSelector;

use super::*;

fn by_name(name: &str) -> SelectorSpec {
    SelectorSpec::Name(name.to_string())
}

async fn start(h: &Harness, id: u64, app: AppConfig) {
    let reply = reply_of(dispatch(envelope(id, Request::Start { apps: vec![app] }), &h.ctx).await);
    assert!(reply.result.is_ok(), "fixture start failed: {reply:?}");
}

async fn disable(h: &Harness, id: u64, name: &str) {
    let reply = reply_of(
        dispatch(
            envelope(
                id,
                Request::Disable {
                    selector: by_name(name),
                },
            ),
            &h.ctx,
        )
        .await,
    );
    assert!(reply.result.is_ok(), "fixture disable failed: {reply:?}");
}

/// The flock must still hold one stopped, disabled `api` that never ran a
/// second process.
async fn assert_parked(h: &Harness, id: u64) {
    let flock = list_flock(&h.ctx, id).await;
    assert_eq!(flock.len(), 1, "{flock:?}");
    assert_eq!(flock[0].status, ProcStatus::Stopped, "{flock:?}");
    assert_eq!(flock[0].disabled, Some(true), "{flock:?}");
    assert_eq!(flock[0].restarts, 0, "{flock:?}");
}

/// An older CLI restarts what it resolved id by id. Fails if a per-id
/// restart of the second instance reaches a respawn.
#[tokio::test(start_paused = true)]
async fn a_per_id_restart_of_a_disabled_app_is_refused_and_names_enable() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    let mut app = AppConfig::minimal("api", "./srv");
    app.instances = 2;
    start(&h, 1, app).await;
    disable(&h, 2, "api").await;

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Restart {
                    selector: SelectorSpec::Id(1),
                },
            ),
            &h.ctx,
        )
        .await,
    );

    let err = reply.result.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::Disabled);
    assert!(err.message.contains("`shep enable api`"), "{}", err.message);
    let flock = list_flock(&h.ctx, 4).await;
    assert!(
        flock.iter().all(|info| info.status == ProcStatus::Stopped),
        "{flock:?}"
    );
}

/// A cron occurrence and a watch trigger both arrive as an automatic restart
/// of the name. Fails if either respawns the app or is answered as a refusal.
#[tokio::test(start_paused = true)]
async fn a_cron_or_watch_restart_leaves_a_disabled_app_stopped() {
    let h = harness(vec![ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, "api").await;

    let swept = h
        .ctx
        .supervisor
        .restart_automatic(ProcessSelector::Name("api".to_string()))
        .await;

    let infos = swept.expect("the shepherd's own restart is dropped, not refused");
    assert!(
        infos.iter().all(|info| info.status == ProcStatus::Stopped),
        "{infos:?}"
    );
    assert_parked(&h, 3).await;
}

/// A memory breach or a liveness failure names the pid it measured. Fails if
/// the report outlives the disable and restarts the app.
#[tokio::test(start_paused = true)]
async fn a_memory_or_liveness_restart_leaves_a_disabled_app_stopped() {
    let h = harness(vec![ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    let pid = list_flock(&h.ctx, 2).await[0].pid.expect("api is online");
    disable(&h, 3, "api").await;

    h.ctx.supervisor.extra_restart(0, pid, None, None).await;

    assert_parked(&h, 4).await;
}

/// The crash ladder's backoff timer is armed when the disable lands. Fails if
/// the timer fires into a respawn.
#[tokio::test(start_paused = true)]
async fn the_crash_ladder_leaves_a_disabled_app_stopped() {
    let h = harness(vec![ProcScript::const_exit(1), ProcScript::never_exits()]);
    let mut app = AppConfig::minimal("api", "./srv");
    app.exp_backoff_restart_delay = Some("500".parse().unwrap());
    start(&h, 1, app).await;
    let mut id = 2;
    while list_flock(&h.ctx, id).await[0].status != ProcStatus::WaitingRestart {
        id += 1;
        tokio::task::yield_now().await;
    }
    disable(&h, id + 1, "api").await;

    tokio::time::sleep(Duration::from_secs(60)).await;

    assert_parked(&h, id + 2).await;
}
