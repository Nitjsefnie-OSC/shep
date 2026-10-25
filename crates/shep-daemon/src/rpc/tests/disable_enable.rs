//! `Disable` and `Enable` by name: the mark belongs to the app, and every
//! fresh spawn of a disabled app is refused with `RpcErrorCode::Disabled`.

use super::*;

fn by_name(name: &str) -> SelectorSpec {
    SelectorSpec::Name(name.to_string())
}

async fn start(h: &Harness, id: u64, app: AppConfig) {
    let reply = reply_of(dispatch(envelope(id, Request::Start { apps: vec![app] }), &h.ctx).await);
    assert!(reply.result.is_ok(), "fixture start failed: {reply:?}");
}

async fn disable(h: &Harness, id: u64, selector: SelectorSpec) -> Reply {
    reply_of(dispatch(envelope(id, Request::Disable { selector }), &h.ctx).await)
}

async fn enable(h: &Harness, id: u64, selector: SelectorSpec) -> Reply {
    reply_of(dispatch(envelope(id, Request::Enable { selector }), &h.ctx).await)
}

fn two_instances(name: &str) -> AppConfig {
    let mut app = AppConfig::minimal(name, "./srv");
    app.instances = 2;
    app
}

#[tokio::test(start_paused = true)]
async fn disable_stops_the_app_and_the_listing_says_stopped_and_disabled() {
    let h = harness(vec![ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;

    let Ok(Response::Disabled(infos)) = disable(&h, 2, by_name("api")).await.result else {
        panic!("expected Disabled")
    };
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].status, ProcStatus::Stopped);
    assert_eq!(infos[0].disabled, Some(true));

    let flock = list_flock(&h.ctx, 3).await;
    assert_eq!(flock.len(), 1);
    assert_eq!(flock[0].status, ProcStatus::Stopped);
    assert_eq!(flock[0].disabled, Some(true));
}

#[tokio::test(start_paused = true)]
async fn an_app_that_was_never_disabled_lists_no_mark() {
    let h = harness(vec![ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;

    assert_eq!(list_flock(&h.ctx, 2).await[0].disabled, None);
}

/// Fails if `do_start` stops consulting the mark: `Start` would register a
/// second `api` instead of refusing.
#[tokio::test(start_paused = true)]
async fn start_of_a_disabled_name_is_refused_and_names_enable() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, by_name("api")).await.result.unwrap();

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Start {
                    apps: vec![AppConfig::minimal("api", "./srv")],
                },
            ),
            &h.ctx,
        )
        .await,
    );

    let err = reply.result.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::Disabled);
    assert!(
        err.message.contains("`shep enable api`"),
        "the refusal must name the way back: {}",
        err.message
    );
    let flock = list_flock(&h.ctx, 4).await;
    assert_eq!(flock.len(), 1, "nothing was registered");
    assert_eq!(flock[0].status, ProcStatus::Stopped);
}

#[tokio::test(start_paused = true)]
async fn start_refuses_a_batch_whole_when_one_app_in_it_is_disabled() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, by_name("api")).await.result.unwrap();

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Start {
                    apps: vec![
                        AppConfig::minimal("worker", "./srv"),
                        AppConfig::minimal("api", "./srv"),
                    ],
                },
            ),
            &h.ctx,
        )
        .await,
    );

    assert_eq!(reply.result.unwrap_err().code, RpcErrorCode::Disabled);
    assert_eq!(list_flock(&h.ctx, 4).await.len(), 1, "worker never started");
}

/// Fails if the respawn path of an operator `Restart` ignores the mark:
/// `shep start api` on a registered, stopped sheep goes through it.
#[tokio::test(start_paused = true)]
async fn restart_of_a_disabled_app_is_refused_and_names_enable() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, by_name("api")).await.result.unwrap();

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Restart {
                    selector: SelectorSpec::Id(0),
                },
            ),
            &h.ctx,
        )
        .await,
    );

    let err = reply.result.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::Disabled);
    assert!(err.message.contains("`shep enable api`"), "{}", err.message);
    assert_eq!(list_flock(&h.ctx, 4).await[0].status, ProcStatus::Stopped);
}

/// Fails if `handle_scale` stops consulting the mark: the scale-up would
/// spawn a second instance of a disabled app.
#[tokio::test(start_paused = true)]
async fn scaling_a_disabled_app_up_is_refused_and_names_enable() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, by_name("api")).await.result.unwrap();

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Scale {
                    name: "api".to_string(),
                    count: 2,
                },
            ),
            &h.ctx,
        )
        .await,
    );

    let err = reply.result.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::Disabled);
    assert!(err.message.contains("`shep enable api`"), "{}", err.message);
    assert_eq!(list_flock(&h.ctx, 4).await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn disabling_by_id_widens_to_every_instance_of_the_app() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, two_instances("api")).await;

    let Ok(Response::Disabled(infos)) = disable(&h, 2, SelectorSpec::Id(1)).await.result else {
        panic!("expected Disabled")
    };

    assert_eq!(infos.len(), 2, "the whole app, not the one instance");
    for info in list_flock(&h.ctx, 3).await {
        assert_eq!(info.status, ProcStatus::Stopped);
        assert_eq!(info.disabled, Some(true));
    }
}

#[tokio::test(start_paused = true)]
async fn disabling_an_unknown_name_is_not_found() {
    let h = harness(vec![]);

    let reply = disable(&h, 1, by_name("ghost")).await;

    assert_eq!(reply.result.unwrap_err().code, RpcErrorCode::NotFound);
}

#[tokio::test(start_paused = true)]
async fn enable_clears_the_mark_and_starts_the_app() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, by_name("api")).await.result.unwrap();

    let Ok(Response::Enabled(infos)) = enable(&h, 3, by_name("api")).await.result else {
        panic!("expected Enabled")
    };

    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].status, ProcStatus::Online);
    assert_eq!(infos[0].disabled, None);
    let flock = list_flock(&h.ctx, 4).await;
    assert_eq!(flock[0].status, ProcStatus::Online);
    assert_eq!(flock[0].disabled, None);
}

#[tokio::test(start_paused = true)]
async fn enabling_an_app_that_is_not_disabled_changes_nothing() {
    let h = harness(vec![ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;

    let reply = enable(&h, 2, by_name("api")).await;

    assert_eq!(reply.result.unwrap(), Response::Enabled(vec![]));
    let flock = list_flock(&h.ctx, 3).await;
    assert_eq!(flock.len(), 1);
    assert_eq!(flock[0].status, ProcStatus::Online);
}

#[tokio::test(start_paused = true)]
async fn enabling_an_unknown_name_is_not_found() {
    let h = harness(vec![]);

    let reply = enable(&h, 1, by_name("ghost")).await;

    assert_eq!(reply.result.unwrap_err().code, RpcErrorCode::NotFound);
}

#[tokio::test(start_paused = true)]
async fn a_start_after_enable_is_accepted_again() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    disable(&h, 2, by_name("api")).await.result.unwrap();
    enable(&h, 3, by_name("api")).await.result.unwrap();

    let reply = reply_of(
        dispatch(
            envelope(
                4,
                Request::Restart {
                    selector: by_name("api"),
                },
            ),
            &h.ctx,
        )
        .await,
    );

    assert!(reply.result.is_ok(), "{reply:?}");
}

/// Fails if the staged walk hands a disabled app to the supervisor by name:
/// an exact selector is refused, and `restart all` would exit non-zero over
/// an app the operator parked on purpose.
#[tokio::test(start_paused = true)]
async fn a_sweep_restart_passes_a_disabled_app_by_without_refusing_it() {
    let h = harness(vec![ProcScript::never_exits(); 3]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;
    start(&h, 2, AppConfig::minimal("worker", "./srv")).await;
    disable(&h, 3, by_name("api")).await.result.unwrap();

    let reply = reply_of(
        dispatch(
            envelope(
                4,
                Request::Restart {
                    selector: SelectorSpec::All,
                },
            ),
            &h.ctx,
        )
        .await,
    );

    let Ok(Response::Restarted { accepted, refused }) = reply.result else {
        panic!("expected Restarted, got {reply:?}")
    };
    assert!(refused.is_empty(), "nothing was refused: {refused:?}");
    let names: Vec<&str> = accepted.iter().map(|info| info.name.as_str()).collect();
    assert_eq!(names, ["worker"]);
    let flock = list_flock(&h.ctx, 5).await;
    assert_eq!(flock[0].name, "api");
    assert_eq!(flock[0].status, ProcStatus::Stopped);
}

/// Fails if an exit carrying an operator `Restart` marker respawns without
/// reading the mark: the disable's stop rode that marker and the app comes
/// back online while marked disabled.
#[tokio::test(start_paused = true)]
async fn disabling_during_a_restart_in_flight_leaves_the_app_stopped() {
    let h = harness(vec![
        ProcScript::ignores_signals(),
        ProcScript::never_exits(),
    ]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;

    // Straight to the supervisor: `dispatch` lists the flock first, which
    // would let the disable reach the actor ahead of the restart.
    let (restarted, disabled) = tokio::join!(
        h.ctx
            .supervisor
            .restart(shep_core::selector::ProcessSelector::Name(
                "api".to_string()
            )),
        disable(&h, 2, by_name("api")),
    );

    restarted.expect("the restart in flight is answered");
    let Ok(Response::Disabled(infos)) = disabled.result else {
        panic!("expected Disabled")
    };
    assert_eq!(infos[0].status, ProcStatus::Stopped);
    let flock = list_flock(&h.ctx, 4).await;
    assert_eq!(flock[0].status, ProcStatus::Stopped);
    assert_eq!(flock[0].disabled, Some(true));
}

/// Fails if an enable rides the stop a disable already started: the mark is
/// cleared and the app ends up stopped, with `Enabled` claiming success.
#[tokio::test(start_paused = true)]
async fn enabling_during_a_disable_in_flight_starts_the_app() {
    let h = harness(vec![
        ProcScript::ignores_signals(),
        ProcScript::never_exits(),
    ]);
    start(&h, 1, AppConfig::minimal("api", "./srv")).await;

    let (disabled, enabled) = tokio::join!(
        disable(&h, 2, by_name("api")),
        enable(&h, 3, by_name("api")),
    );

    assert!(disabled.result.is_ok());
    let Ok(Response::Enabled(infos)) = enabled.result else {
        panic!("expected Enabled")
    };
    assert_eq!(infos[0].status, ProcStatus::Online);
    let flock = list_flock(&h.ctx, 4).await;
    assert_eq!(flock[0].status, ProcStatus::Online);
    assert_eq!(flock[0].disabled, None);
}
