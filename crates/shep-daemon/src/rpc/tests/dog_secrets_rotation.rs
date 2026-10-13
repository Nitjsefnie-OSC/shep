//! `config.dog.<name>`: fired when a provider's push changes a value a dog's
//! section names, and never when it does not.

use super::*;
use crate::bus::SharedEvent;
use crate::testing::capture_logs;
use shep_core::protocol::BusEvent;
use tokio::sync::broadcast;

/// A value no frame, message or log line may carry.
const SENTINEL: &str = "sentinel-7f3a9c";

/// `bark` and `metrics` name `vercel/TOKEN` and `vercel/OTHER`; `quiet`
/// names a different namespace.
const DOGS: &str = "\
[bark]
token = \"{{secret:vercel/TOKEN}}\"

[metrics.auth]
key = \"{{secret:vercel/TOKEN}}\"

[quiet]
token = \"{{secret:elsewhere/TOKEN}}\"
";

/// Sends one `PutSecrets` and returns the reply's result.
async fn push_in(
    ctx: &RpcContext,
    namespace: &str,
    environment: &str,
    entries: &[(&str, &str)],
) -> Result<Response, shep_core::protocol::RpcError> {
    reply_of(
        dispatch(
            envelope(
                1,
                Request::PutSecrets {
                    namespace: namespace.to_string(),
                    environment: environment.to_string(),
                    entries: entries
                        .iter()
                        .map(|(key, value)| ((*key).to_string(), (*value).to_string().into()))
                        .collect(),
                },
            ),
            ctx,
        )
        .await,
    )
    .result
}

async fn push(ctx: &RpcContext, entries: &[(&str, &str)]) {
    push_in(ctx, "vercel", "production", entries)
        .await
        .expect("the push lands");
}

/// Every `DogConfigChanged` queued on `rx`, as its topic.
fn topics(rx: &mut broadcast::Receiver<SharedEvent>) -> Vec<String> {
    frames(rx)
        .iter()
        .map(|event| event.topic().to_string())
        .collect()
}

fn frames(rx: &mut broadcast::Receiver<SharedEvent>) -> Vec<BusEvent> {
    let mut heard = Vec::new();
    while let Ok(event) = rx.try_recv() {
        let event = event.to_event();
        if matches!(event, BusEvent::DogConfigChanged { .. }) {
            heard.push(event);
        }
    }
    heard
}

#[tokio::test(start_paused = true)]
async fn a_changed_value_fires_one_event_per_dog_that_names_it() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    push(&h.ctx, &[("TOKEN", "old")]).await;
    let mut rx = h.ctx.events.subscribe();

    push(&h.ctx, &[("TOKEN", "new")]).await;

    assert_eq!(
        topics(&mut rx),
        ["config.dog.bark", "config.dog.metrics"],
        "sorted by name, and nothing for quiet"
    );
}

#[tokio::test(start_paused = true)]
async fn a_re_push_of_the_same_values_fires_nothing() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    push(&h.ctx, &[("TOKEN", "same")]).await;
    let mut rx = h.ctx.events.subscribe();

    push(&h.ctx, &[("TOKEN", "same")]).await;

    assert_eq!(topics(&mut rx), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn a_push_for_another_environment_fires_nothing() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    push(&h.ctx, &[("TOKEN", "old")]).await;
    let mut rx = h.ctx.events.subscribe();

    push_in(&h.ctx, "vercel", "staging", &[("TOKEN", "new")])
        .await
        .unwrap();

    assert_eq!(topics(&mut rx), Vec::<String>::new());
}

#[tokio::test(start_paused = true)]
async fn the_first_push_of_a_namespace_a_dog_named_fires() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    let mut rx = h.ctx.events.subscribe();

    push(&h.ctx, &[("TOKEN", "first")]).await;

    assert_eq!(topics(&mut rx), ["config.dog.bark", "config.dog.metrics"]);
}

#[tokio::test(start_paused = true)]
async fn a_key_the_push_drops_fires() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    push(&h.ctx, &[("TOKEN", "t"), ("KEEP", "k")]).await;
    let mut rx = h.ctx.events.subscribe();

    push(&h.ctx, &[("KEEP", "k")]).await;

    assert_eq!(topics(&mut rx), ["config.dog.bark", "config.dog.metrics"]);
}

#[tokio::test(start_paused = true)]
async fn a_key_no_dog_names_fires_nothing() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    push(&h.ctx, &[("TOKEN", "t"), ("UNNAMED", "old")]).await;
    let mut rx = h.ctx.events.subscribe();

    push(&h.ctx, &[("TOKEN", "t"), ("UNNAMED", "new")]).await;

    assert_eq!(topics(&mut rx), Vec::<String>::new());
}

/// The values are live in memory when `put` returns `Err`, so a dog told
/// nothing would keep the secret that was just rotated away.
#[tokio::test(start_paused = true)]
async fn a_failed_cache_write_still_fires() {
    let h = harness(vec![]);
    std::fs::write(&h.ctx.dogs_config, DOGS).unwrap();
    push(&h.ctx, &[("TOKEN", "old")]).await;
    let mut rx = h.ctx.events.subscribe();
    // A directory where the cache file goes: the `rename` onto it fails.
    std::fs::remove_file(&h.ctx.paths.secrets_cache).unwrap();
    std::fs::create_dir(&h.ctx.paths.secrets_cache).unwrap();

    let refused = push_in(&h.ctx, "vercel", "production", &[("TOKEN", "new")])
        .await
        .unwrap_err();

    assert_eq!(refused.code, RpcErrorCode::Internal, "{refused:?}");
    assert_eq!(topics(&mut rx), ["config.dog.bark", "config.dog.metrics"]);
}

/// A provider that reads its own namespace re-asks on the event and pushes
/// what it already holds. That second push must be silent or it never ends.
#[tokio::test(start_paused = true)]
async fn a_provider_that_names_its_own_namespace_does_not_loop() {
    let h = harness(vec![]);
    std::fs::write(
        &h.ctx.dogs_config,
        "[vercel]\nsecret_id = \"{{secret:vercel/SECRET_ID}}\"\n",
    )
    .unwrap();
    let mut rx = h.ctx.events.subscribe();

    push(&h.ctx, &[("SECRET_ID", "rotated")]).await;
    assert_eq!(topics(&mut rx), ["config.dog.vercel"]);

    push(&h.ctx, &[("SECRET_ID", "rotated")]).await;
    assert_eq!(topics(&mut rx), Vec::<String>::new());
}

/// One thread, one `block_on`: `capture_logs` scopes its subscriber to a
/// synchronous closure.
#[test]
fn the_frame_carries_the_dog_name_and_no_value() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap();
    let mut heard = Vec::new();
    let logs = capture_logs(|| {
        rt.block_on(async {
            let h = harness(vec![]);
            std::fs::write(
                &h.ctx.dogs_config,
                "[paddock]\nowner = \"{{secret:vercel/TOKEN}}\"\n",
            )
            .unwrap();
            let mut rx = h.ctx.events.subscribe();
            push(&h.ctx, &[("TOKEN", SENTINEL)]).await;
            heard = frames(&mut rx);
        });
    });

    let json: Vec<String> = heard
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect();
    assert_eq!(
        json,
        [r#"{"event":"dog_config_changed","data":{"dog":"paddock"}}"#]
    );
    for rendered in [format!("{heard:?}"), logs] {
        assert!(!rendered.contains(SENTINEL), "{rendered}");
    }
}
