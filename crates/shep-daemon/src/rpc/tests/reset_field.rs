//! `ResetSheepField`: an override dropped, the value it shadowed back in the
//! flock, and the refusals it shares with `SetSheepField`.

use super::*;

/// Sends one request and hands back its result.
async fn send(ctx: &RpcContext, id: u64, body: Request) -> Result<Response, RpcError> {
    reply_of(dispatch(envelope(id, body), ctx).await).result
}

fn set(key: &str, value: serde_json::Value) -> Request {
    Request::SetSheepField {
        name: "web".to_string(),
        key: key.to_string(),
        value,
    }
}

fn reset(key: &str) -> Request {
    Request::ResetSheepField {
        name: "web".to_string(),
        key: key.to_string(),
    }
}

/// A Flockfile load of `web` declaring `max_restarts` as `value`.
fn load(value: u32, reset: ResetDepth) -> Request {
    let mut config = AppConfig::minimal("web", "./srv");
    config.max_restarts = value;
    Request::ApplyConfig {
        apps: vec![DeclaredApp {
            config,
            declared: ["max_restarts"]
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            declared_env: BTreeSet::new(),
        }],
        reset,
    }
}

/// fails if a reset does not bring back the value the sheep held before the
/// operator spoke for the key, in the running config, in the `*` marker, and
/// in the store a restart would read.
#[tokio::test(start_paused = true)]
async fn a_reset_brings_back_the_value_the_override_shadowed() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;
    let before = sheep_config_view(&h.ctx, 2, "web")
        .await
        .config
        .max_restarts;

    send(&h.ctx, 3, set("max_restarts", serde_json::json!(40)))
        .await
        .unwrap();
    assert_eq!(
        sheep_config_view(&h.ctx, 4, "web")
            .await
            .config
            .max_restarts,
        40
    );

    let reply = send(&h.ctx, 5, reset("max_restarts")).await.unwrap();
    let Response::SheepFieldReset {
        key,
        value,
        pending,
        ..
    } = reply
    else {
        panic!("expected SheepFieldReset, got {reply:?}");
    };
    assert_eq!(key, "max_restarts");
    assert_eq!(value, serde_json::json!(before));
    assert!(!pending, "max_restarts is Live and is in force now");

    let view = sheep_config_view(&h.ctx, 6, "web").await;
    assert_eq!(view.config.max_restarts, before);
    assert!(view.overridden.is_empty(), "{:?}", view.overridden);
    let stored = shep_core::overrides::get(&h.ctx.paths.overrides, "web")
        .unwrap()
        .unwrap();
    assert!(!stored.fields.contains_key("max_restarts"), "{stored:?}");
    assert!(!stored.template.contains_key("max_restarts"), "{stored:?}");
    let roll = h.ctx.registry.roll(&list_flock(&h.ctx, 7).await, 0);
    assert_eq!(
        roll.apps[0].app.max_restarts, before,
        "a cold restart reads the roll"
    );
}

/// fails if a reset returns to a stale value. The Flockfile changed after the
/// operator's override and a load left the sheep alone, which is what a load
/// does to a key the operator holds; the value that returns is the file's now.
#[tokio::test(start_paused = true)]
async fn a_reset_returns_to_what_the_flockfile_says_now() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;
    send(&h.ctx, 2, load(7, ResetDepth::File)).await.unwrap();
    send(&h.ctx, 3, set("max_restarts", serde_json::json!(40)))
        .await
        .unwrap();

    send(&h.ctx, 4, load(9, ResetDepth::None)).await.unwrap();
    assert_eq!(
        sheep_config_view(&h.ctx, 5, "web")
            .await
            .config
            .max_restarts,
        40,
        "a plain load leaves the operator's value alone"
    );

    send(&h.ctx, 6, reset("max_restarts")).await.unwrap();
    assert_eq!(
        sheep_config_view(&h.ctx, 7, "web")
            .await
            .config
            .max_restarts,
        9
    );
}

/// fails if a second edit of an overridden key replaces the remembered value
/// with the first edit's. The template is what the key held without the
/// operator, so it is recorded once.
#[tokio::test(start_paused = true)]
async fn editing_an_overridden_key_again_keeps_the_original_template() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;
    let before = sheep_config_view(&h.ctx, 2, "web")
        .await
        .config
        .max_restarts;

    send(&h.ctx, 3, set("max_restarts", serde_json::json!(40)))
        .await
        .unwrap();
    send(&h.ctx, 4, set("max_restarts", serde_json::json!(50)))
        .await
        .unwrap();
    send(&h.ctx, 5, reset("max_restarts")).await.unwrap();

    assert_eq!(
        sheep_config_view(&h.ctx, 6, "web")
            .await
            .config
            .max_restarts,
        before
    );
}

/// fails if a record written before templates were kept cannot be reset.
/// It goes back to the built-in default, the value a pane's `d` restores.
#[tokio::test(start_paused = true)]
async fn a_record_with_no_template_goes_back_to_the_default() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;
    let record = shep_core::overrides::AppOverrides {
        fields: serde_json::Map::from_iter([("max_restarts".to_string(), serde_json::json!(40))]),
        ..Default::default()
    };
    shep_core::overrides::put(&h.ctx.paths.overrides, "web", &record).unwrap();
    send(&h.ctx, 2, set("max_restarts", serde_json::json!(41)))
        .await
        .unwrap();
    // The write above recorded a template: drop it to model the older record.
    shep_core::overrides::put(&h.ctx.paths.overrides, "web", &record).unwrap();

    send(&h.ctx, 3, reset("max_restarts")).await.unwrap();

    assert_eq!(
        sheep_config_view(&h.ctx, 4, "web")
            .await
            .config
            .max_restarts,
        AppConfig::default().max_restarts
    );
}

/// fails if a reset of a key nobody overrode reads as success. Nothing was
/// dropped, and an operator who typed the wrong key should be told.
#[tokio::test(start_paused = true)]
async fn a_key_with_no_override_is_refused() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;

    let err = send(&h.ctx, 2, reset("max_restarts")).await.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert!(err.message.contains("no override"), "{err:?}");
}

/// fails if a reset reaches a key `SetSheepField` leaves to another request.
#[tokio::test(start_paused = true)]
async fn the_keys_set_refuses_are_refused_here_too() {
    let h = harness(vec![ProcScript::never_exits()]);
    start_web(&h).await;

    for (id, key) in [(2, "env"), (3, "dogs"), (4, "instances"), (5, "name")] {
        let err = send(&h.ctx, id, reset(key)).await.unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidConfig, "{key}: {err:?}");
    }
}

/// fails if an unknown sheep is anything but NotFound.
#[tokio::test(start_paused = true)]
async fn an_unknown_sheep_is_not_found() {
    let h = harness(vec![]);
    let err = send(&h.ctx, 1, reset("max_restarts")).await.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::NotFound);
}
