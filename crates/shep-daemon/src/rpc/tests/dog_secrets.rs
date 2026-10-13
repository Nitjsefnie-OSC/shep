//! `DogConfigResolved`: a dog's section with its `{{secret:...}}` references
//! replaced, beside `DogConfig` serving the same section as written.

use super::*;
use crate::testing::capture_logs;
use shep_core::protocol::RpcError;
use shep_core::secrets::ALL_ENVIRONMENTS;

/// A value no message, `Debug` rendering or log line may carry.
const SENTINEL: &str = "sentinel-7f3a9c";

const PADDOCK: &str = "\
[paddock.client_keys]
owner = \"{{secret:paddock/owner}}\"
";

async fn resolved(ctx: &RpcContext, name: &str) -> Result<String, RpcError> {
    let reply = reply_of(
        dispatch(
            envelope(
                1,
                Request::DogConfigResolved {
                    name: name.to_string(),
                },
            ),
            ctx,
        )
        .await,
    );
    match reply.result {
        Ok(Response::DogSection { toml }) => Ok(toml.as_str().to_string()),
        Ok(other) => panic!("expected DogSection, got {other:?}"),
        Err(err) => Err(err),
    }
}

async fn plain(ctx: &RpcContext, name: &str) -> String {
    let reply = reply_of(
        dispatch(
            envelope(
                2,
                Request::DogConfig {
                    name: name.to_string(),
                },
            ),
            ctx,
        )
        .await,
    );
    let Ok(Response::DogSection { toml }) = reply.result else {
        panic!("expected DogSection, got {:?}", reply.result)
    };
    toml.as_str().to_string()
}

fn push(ctx: &RpcContext, namespace: &str, environment: &str, pairs: &[(&str, &str)]) {
    let entries = pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    ctx.provider_secrets
        .put(namespace, environment, entries, false)
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_resolved_read_serves_the_value_and_the_plain_read_still_serves_the_reference() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, PADDOCK).unwrap();
    push(&h.ctx, "paddock", "production", &[("owner", "ada-key")]);

    assert_eq!(
        resolved(&h.ctx, "paddock").await.unwrap(),
        "[client_keys]\nowner = \"ada-key\"\n"
    );
    assert_eq!(
        plain(&h.ctx, "paddock").await,
        "[client_keys]\nowner = \"{{secret:paddock/owner}}\"\n"
    );
}

#[tokio::test(start_paused = true)]
async fn serving_a_resolved_section_never_rewrites_dogs_toml() {
    let h = harness(vec![]);
    let source = format!("{PADDOCK}\n[bark]\ntoken = \"{{{{secret:TOKEN}}}}\" # note\n");
    std::fs::write(&h.ctx.dogs_config, &source).unwrap();
    push(&h.ctx, "paddock", "production", &[("owner", "ada-key")]);
    shep_core::secrets::set(&h.ctx.paths.secrets, "OTHER", "production", SENTINEL).unwrap();

    resolved(&h.ctx, "paddock").await.unwrap();
    assert_eq!(std::fs::read_to_string(&h.ctx.dogs_config).unwrap(), source);

    resolved(&h.ctx, "bark").await.unwrap_err();
    assert_eq!(std::fs::read_to_string(&h.ctx.dogs_config).unwrap(), source);
}

/// One thread, one `block_on`: `capture_logs` scopes its subscriber to a
/// synchronous closure.
#[test]
fn a_refusal_names_the_key_path_and_the_reference_and_no_value() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let mut outcome = None;
    let logs = capture_logs(|| {
        rt.block_on(async {
            let h = harness(vec![]);
            // The namespace is up and holds a value, but not this key.
            push(&h.ctx, "paddock", "production", &[("other", SENTINEL)]);
            shep_core::secrets::set(&h.ctx.paths.secrets, "OTHER", "production", SENTINEL).unwrap();

            std::fs::write(
                &h.ctx.dogs_config,
                format!("{PADDOCK}\n[bark]\ntoken = \"{{{{secret:OTHER}}}}\"\n"),
            )
            .unwrap();
            let served = resolved(&h.ctx, "bark").await;
            let refused = resolved(&h.ctx, "paddock").await;
            outcome = Some((served, refused));
        });
    });
    let (served, refused) = outcome.unwrap();
    assert_eq!(served.unwrap(), format!("token = \"{SENTINEL}\"\n"));
    let err = refused.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert_eq!(
        err.message,
        "[paddock] `client_keys.owner`: `{{secret:paddock/owner}}` has no value in the \
         `production` environment"
    );
    for rendered in [err.message.clone(), format!("{err:?}"), logs] {
        assert!(!rendered.contains(SENTINEL), "{rendered}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_section_with_no_reference_is_served_even_when_the_store_is_corrupt() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, "[bark]\npoll = \"5s\"\n").unwrap();
    std::fs::write(&h.ctx.paths.secrets, "{ not json").unwrap();

    assert_eq!(resolved(&h.ctx, "bark").await.unwrap(), "poll = \"5s\"\n");

    std::fs::write(&h.ctx.dogs_config, "[bark]\ntoken = \"{{secret:T}}\"\n").unwrap();
    let err = resolved(&h.ctx, "bark").await.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert_eq!(
        err.message,
        "[bark] `token`: the secret store could not be read (its contents do not parse)"
    );
}

#[tokio::test(start_paused = true)]
async fn a_dog_resolves_in_the_host_environment_with_the_all_fallback_and_never_another_environment()
 {
    let mut h = harness(vec![]);
    h.ctx.host_environment = "staging".to_string();
    std::fs::write(&h.ctx.dogs_config, "[bark]\ntoken = \"{{secret:TOKEN}}\"\n").unwrap();

    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "production", SENTINEL).unwrap();
    let err = resolved(&h.ctx, "bark").await.unwrap_err();
    assert_eq!(
        err.message,
        "[bark] `token`: `{{secret:TOKEN}}` has no value in the `staging` environment"
    );
    assert!(!err.message.contains(SENTINEL));

    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", ALL_ENVIRONMENTS, "shared").unwrap();
    assert_eq!(
        resolved(&h.ctx, "bark").await.unwrap(),
        "token = \"shared\"\n"
    );

    shep_core::secrets::set(&h.ctx.paths.secrets, "TOKEN", "staging", "mine").unwrap();
    assert_eq!(
        resolved(&h.ctx, "bark").await.unwrap(),
        "token = \"mine\"\n"
    );
}

#[tokio::test(start_paused = true)]
async fn a_provider_namespace_that_has_not_pushed_is_named_as_unready() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, PADDOCK).unwrap();

    let err = resolved(&h.ctx, "paddock").await.unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig);
    assert_eq!(
        err.message,
        "[paddock] `client_keys.owner`: `{{secret:paddock/owner}}` reads the `paddock` \
         namespace, which no provider dog has pushed to for the `production` environment yet"
    );
}
