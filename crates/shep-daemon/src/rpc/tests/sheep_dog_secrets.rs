//! `SheepDogSettingsResolved`: one sheep's per-dog table with its
//! `{{secret:...}}` references replaced in that sheep's own environment,
//! beside `DogSheepSettings` serving the same table as written.

use super::dog_tables::{carrying, start, table};
use super::*;
use crate::testing::{capture_logs, harness_in_environment};
use shep_core::config::DogTable;
use shep_core::protocol::RpcError;
use shep_core::secrets::ALL_ENVIRONMENTS;

/// A value no message, `Debug` rendering or log line may carry.
const SENTINEL: &str = "sentinel-7f3a9c";

/// A sheep in `environment` (the host's when `None`) carrying a `deploy`
/// table.
fn deploying(name: &str, environment: Option<&str>, deploy: DogTable) -> AppConfig {
    let mut app = carrying(name, vec![("deploy", deploy)]);
    app.environment = environment.map(str::to_string);
    app
}

fn token_table() -> DogTable {
    table(serde_json::json!({
        "env": { "TOKEN": "{{secret:TOKEN}}" },
        "retries": 3,
    }))
}

async fn resolved(ctx: &RpcContext, name: &str, dog: &str) -> Result<Option<DogTable>, RpcError> {
    let reply = reply_of(
        dispatch(
            envelope(
                1,
                Request::SheepDogSettingsResolved {
                    name: name.to_string(),
                    dog: dog.to_string(),
                },
            ),
            ctx,
        )
        .await,
    );
    match reply.result {
        Ok(Response::SheepDogTable { table }) => Ok(table),
        Ok(other) => panic!("expected SheepDogTable, got {other:?}"),
        Err(err) => Err(err),
    }
}

async fn plain(ctx: &RpcContext, dog: &str) -> BTreeMap<String, DogTable> {
    let reply = reply_of(
        dispatch(
            envelope(
                2,
                Request::DogSheepSettings {
                    dog: dog.to_string(),
                },
            ),
            ctx,
        )
        .await,
    );
    match reply.result {
        Ok(Response::DogSheepSettings { tables }) => tables,
        other => panic!("expected DogSheepSettings, got {other:?}"),
    }
}

fn as_json(table: Option<DogTable>) -> serde_json::Value {
    serde_json::to_value(table.expect("the sheep carries a table")).unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_reference_resolves_in_each_sheeps_own_environment() {
    let h = harness(vec![ProcScript::never_exits(); 2]);
    start(
        &h.ctx,
        1,
        vec![
            deploying("web-staging", Some("staging"), token_table()),
            deploying("web-live", Some("production"), token_table()),
        ],
    )
    .await;
    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "staging", "staging-token").unwrap();
    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "production", "live-token").unwrap();

    assert_eq!(
        as_json(resolved(&h.ctx, "web-staging", "deploy").await.unwrap()),
        serde_json::json!({ "env": { "TOKEN": "staging-token" }, "retries": 3 })
    );
    assert_eq!(
        as_json(resolved(&h.ctx, "web-live", "deploy").await.unwrap()),
        serde_json::json!({ "env": { "TOKEN": "live-token" }, "retries": 3 })
    );
}

#[tokio::test(start_paused = true)]
async fn a_sheep_with_no_environment_resolves_in_the_hosts() {
    let h = harness_in_environment(vec![ProcScript::never_exits()], "staging");
    start(&h.ctx, 1, vec![deploying("web", None, token_table())]).await;
    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "production", SENTINEL).unwrap();
    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", ALL_ENVIRONMENTS, "shared").unwrap();

    assert_eq!(
        as_json(resolved(&h.ctx, "web", "deploy").await.unwrap())["env"]["TOKEN"],
        "shared"
    );

    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "staging", "mine").unwrap();
    assert_eq!(
        as_json(resolved(&h.ctx, "web", "deploy").await.unwrap())["env"]["TOKEN"],
        "mine"
    );
}

/// One thread, one `block_on`: `capture_logs` scopes its subscriber to a
/// synchronous closure.
#[test]
fn a_missing_key_is_refused_naming_the_sheep_the_dog_the_key_path_and_the_reference_and_no_value() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let mut refused = None;
    let logs = capture_logs(|| {
        rt.block_on(async {
            let h = harness(vec![ProcScript::never_exits()]);
            start(
                &h.ctx,
                1,
                vec![deploying("web", Some("staging"), token_table())],
            )
            .await;
            // The key exists, but only in another environment.
            shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "production", SENTINEL).unwrap();
            refused = Some(resolved(&h.ctx, "web", "deploy").await);
        });
    });
    let err = refused.unwrap().unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert_eq!(
        err.message,
        "[web] `dogs.deploy.env.TOKEN`: `{{secret:TOKEN}}` has no value in the `staging` \
         environment"
    );
    for rendered in [err.message.clone(), format!("{err:?}"), logs] {
        assert!(!rendered.contains(SENTINEL), "{rendered}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_malformed_reference_is_refused() {
    let h = harness(vec![ProcScript::never_exits()]);
    let malformed = table(serde_json::json!({ "env": { "TOKEN": "{{secret:bad key}}" } }));
    start(&h.ctx, 1, vec![deploying("web", None, malformed)]).await;

    let err = resolved(&h.ctx, "web", "deploy").await.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert_eq!(
        err.message,
        "[web] `dogs.deploy.env.TOKEN`: `{{secret:bad key}}` is not a valid secret reference: \
         write `{{secret:KEY}}` or `{{secret:namespace/KEY}}`"
    );
}

#[tokio::test(start_paused = true)]
async fn a_table_with_no_references_is_answered_over_an_unreadable_store() {
    let h = harness(vec![ProcScript::never_exits(); 2]);
    let plain_table = table(serde_json::json!({ "env": { "MODE": "fast" }, "retries": 3 }));
    start(
        &h.ctx,
        1,
        vec![
            deploying("calm", None, plain_table.clone()),
            deploying("web", None, token_table()),
        ],
    )
    .await;
    std::fs::write(&h.ctx.paths.secrets, "{ not json").unwrap();

    assert_eq!(
        resolved(&h.ctx, "calm", "deploy").await.unwrap(),
        Some(plain_table)
    );

    let err = resolved(&h.ctx, "web", "deploy").await.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert_eq!(
        err.message,
        "[web] `dogs.deploy.env.TOKEN`: the secret store could not be read (its contents do \
         not parse)"
    );
}

#[tokio::test(start_paused = true)]
async fn one_sheeps_bad_reference_does_not_refuse_the_other_sheeps_table() {
    let h = harness(vec![ProcScript::never_exits(); 2]);
    start(
        &h.ctx,
        1,
        vec![
            deploying("good", None, token_table()),
            deploying(
                "bad",
                None,
                table(serde_json::json!({ "k": "{{secret:ABSENT}}" })),
            ),
        ],
    )
    .await;
    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "production", "ok").unwrap();

    resolved(&h.ctx, "bad", "deploy").await.unwrap_err();
    assert_eq!(
        as_json(resolved(&h.ctx, "good", "deploy").await.unwrap())["env"]["TOKEN"],
        "ok"
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_sheep_is_not_found_and_a_sheep_without_the_table_answers_none() {
    let h = harness(vec![ProcScript::never_exits(); 3]);
    start(
        &h.ctx,
        1,
        vec![
            carrying("bare", Vec::new()),
            deploying("web", None, token_table()),
        ],
    )
    .await;
    enable_dog(&h.ctx, 2, "bark").await;

    let missing = resolved(&h.ctx, "ghost", "deploy").await.unwrap_err();
    assert_eq!(missing.code, RpcErrorCode::NotFound);
    assert_eq!(missing.message, "no sheep named ghost");

    let a_dog = resolved(&h.ctx, "bark", "deploy").await.unwrap_err();
    assert_eq!(a_dog.code, RpcErrorCode::NotFound);

    assert_eq!(resolved(&h.ctx, "bare", "deploy").await.unwrap(), None);
    assert_eq!(resolved(&h.ctx, "web", "other").await.unwrap(), None);
}

#[tokio::test(start_paused = true)]
async fn the_plain_read_still_serves_the_reference() {
    let h = harness(vec![ProcScript::never_exits()]);
    start(
        &h.ctx,
        1,
        vec![deploying("web", Some("staging"), token_table())],
    )
    .await;
    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "staging", SENTINEL).unwrap();

    resolved(&h.ctx, "web", "deploy").await.unwrap();
    assert_eq!(
        plain(&h.ctx, "deploy").await,
        BTreeMap::from([("web".to_string(), token_table())])
    );
}
