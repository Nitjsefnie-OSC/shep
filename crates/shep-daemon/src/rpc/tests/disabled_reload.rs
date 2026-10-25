//! A reload against a disabled app is refused over the wire.

use super::*;

/// Fails if `handle_reload` stops consulting the mark: the reply would be an
/// acceptance, with a replacement spawned under a new id.
#[tokio::test(start_paused = true)]
async fn reload_of_a_disabled_app_is_refused_and_names_enable() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    let started = reply_of(
        dispatch(
            envelope(
                1,
                Request::Start {
                    apps: vec![AppConfig::minimal("api", "./srv")],
                },
            ),
            &h.ctx,
        )
        .await,
    );
    assert!(started.result.is_ok(), "fixture start failed: {started:?}");
    let disabled = reply_of(
        dispatch(
            envelope(
                2,
                Request::Disable {
                    selector: SelectorSpec::Name("api".to_string()),
                },
            ),
            &h.ctx,
        )
        .await,
    );
    assert!(
        disabled.result.is_ok(),
        "fixture disable failed: {disabled:?}"
    );

    let reply = reply_of(
        dispatch(
            envelope(
                3,
                Request::Reload {
                    selector: SelectorSpec::Name("api".to_string()),
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
    assert_eq!(flock.len(), 1, "{flock:?}");
    assert_eq!(flock[0].status, ProcStatus::Stopped, "{flock:?}");
}
