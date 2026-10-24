//! The ask, against a scripted shepherd: what a dog's refusal hands back,
//! what the trigger carries, and the three cases with nobody to ask.

use std::fmt::Write as _;
use std::path::Path;

use shep_client::testing::{control_address, fake_client_answering_refusals};
use shep_client::{Client, RequestError};
use shep_core::dogs::validate::VALIDATE_ACTION;
use shep_core::paths::ShepPaths;
use shep_core::protocol::{
    ActionOutcome, ActionReply, Envelope, Request, Response, RpcError, RpcErrorCode, SelectorSpec,
};

use super::*;

/// The dog `kelpie`, adopted at `binary`, with `manifest` cached for it.
///
/// The cache is written by hand rather than by probing a binary: this is the
/// half that decides whether to ask, and a cached manifest is exactly what
/// `shep set` reads on a dog `shep adopt` already read.
fn home_with(dir: &Path, manifest: Option<&str>) -> ShepPaths {
    let paths = ShepPaths::resolve(&|_| None, dir);
    std::fs::create_dir_all(&paths.home).unwrap();
    let binary = dir.join("shep-kelpie");
    std::fs::write(&binary, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        &paths.daemon_config,
        format!(
            "[daemon]\nenabled_dogs = [\"kelpie\"]\n\n[daemon.adopted_dogs]\nkelpie = \"{}\"\n",
            binary.display()
        ),
    )
    .unwrap();
    if let Some(manifest) = manifest {
        let mut context = ring::digest::Context::new(&ring::digest::SHA256);
        context.update(&std::fs::read(&binary).unwrap());
        let digest = context.finish();
        let hex = digest.as_ref().iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
        let dir = paths.home.join("manifests");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{hex}.json")), manifest).unwrap();
    }
    paths
}

/// A manifest declaring `validate`, which is the only manifest that gets asked.
fn validating() -> &'static str {
    r#"{"manifest":1,"name":"kelpie","version":"0.2.0","protocol":12,"channel":true,"validate":true}"#
}

fn table(pairs: &[(&str, serde_json::Value)]) -> serde_json::Map<String, serde_json::Value> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), value.clone()))
        .collect()
}

fn trigger_reply(outcome: ActionOutcome) -> impl Fn(&Request) -> Result<Response, RpcError> {
    move |request| match request {
        Request::Trigger { .. } => Ok(Response::Triggered(vec![ActionReply {
            id: 7,
            name: "kelpie".to_string(),
            outcome: outcome.clone(),
        }])),
        _other => Ok(Response::Pong),
    }
}

/// A daemon that answers the ask with `outcome`, and the envelopes it saw.
async fn asked(
    dir: &Path,
    manifest: Option<&str>,
    outcome: ActionOutcome,
) -> (
    Client,
    ShepPaths,
    tokio::sync::mpsc::UnboundedReceiver<Envelope>,
) {
    let paths = home_with(dir, manifest);
    let path = control_address(dir);
    let (client, envelopes) = fake_client_answering_refusals(&path, trigger_reply(outcome)).await;
    (client, paths, envelopes)
}

/// The refusal, in the dog's own words, is what the caller reports.
#[tokio::test]
async fn a_dog_that_refuses_hands_its_reason_to_the_operator() {
    let dir = tempfile::tempdir().unwrap();
    let (client, paths, _envelopes) = asked(
        dir.path(),
        Some(validating()),
        ActionOutcome::Replied {
            body: r#"{"accept":false,"reason":"this checkout belongs to another project"}"#
                .to_string(),
        },
    )
    .await;

    let failure = ask_dog(
        &client,
        &paths,
        "kelpie",
        &table(&[("checkout", serde_json::json!("/other"))]),
        None,
    )
    .await
    .expect_err("a refusal is a refusal");

    assert_eq!(
        failure,
        AskFailure::Refused {
            reason: "this checkout belongs to another project".to_string()
        }
    );
    assert_eq!(
        failure.request_error(),
        RequestError::Rpc(RpcError {
            code: RpcErrorCode::InvalidConfig,
            message: "this checkout belongs to another project".to_string(),
            daemon_version: None,
        }),
        "the dashboard reports a dog's refusal as a refused write"
    );
}

/// The dog is asked about the table the write would leave behind, by name,
/// over the one action shep defines.
#[tokio::test]
async fn the_ask_carries_the_table_the_write_would_put_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let (client, paths, mut envelopes) = asked(
        dir.path(),
        Some(validating()),
        ActionOutcome::Replied {
            body: r#"{"accept":true}"#.to_string(),
        },
    )
    .await;

    ask_dog(
        &client,
        &paths,
        "kelpie",
        &table(&[("allowed_domains", serde_json::json!(["example.com"]))]),
        Some("api"),
    )
    .await
    .expect("the dog accepted");

    let envelope = envelopes.recv().await.expect("the ask went out");
    let Request::Trigger {
        selector,
        action,
        params,
    } = envelope.body
    else {
        panic!("a dog is asked over a trigger: {:?}", envelope.body);
    };
    assert_eq!(selector, SelectorSpec::Name("kelpie".to_string()));
    assert_eq!(action, VALIDATE_ACTION);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&params.expect("a table is sent")).unwrap(),
        serde_json::json!({
            "table": { "allowed_domains": ["example.com"] },
            "sheep": "api",
        })
    );
}

/// Silence is not acceptance: the dog that was asked and answered nothing has
/// not cleared the write.
#[tokio::test]
async fn a_dog_that_answers_nothing_in_time_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let (client, paths, _envelopes) =
        asked(dir.path(), Some(validating()), ActionOutcome::TimedOut).await;

    let failure = ask_dog(&client, &paths, "kelpie", &table(&[]), None)
        .await
        .expect_err("a timeout is a refusal");

    let AskFailure::Refused { reason } = failure else {
        panic!("a timeout is a refusal: {failure:?}");
    };
    assert!(reason.contains("`kelpie`"), "{reason}");
    assert!(reason.contains("action_timeout"), "{reason}");
}

/// An answer that is not a verdict has not accepted the write either.
#[tokio::test]
async fn a_dog_that_answers_something_else_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let (client, paths, _envelopes) = asked(
        dir.path(),
        Some(validating()),
        ActionOutcome::Replied {
            body: "ok".to_string(),
        },
    )
    .await;

    let failure = ask_dog(&client, &paths, "kelpie", &table(&[]), None)
        .await
        .expect_err("an answer that is not a verdict is not an acceptance");

    let AskFailure::Refused { reason } = failure else {
        panic!("{failure:?}");
    };
    assert!(reason.contains("cannot read"), "{reason}");
    assert!(reason.contains("ok"), "{reason}");
}

/// The acceptance criterion in its plainest form: a dog that accepts lets the
/// write through.
#[tokio::test]
async fn a_dog_that_accepts_clears_the_write() {
    let dir = tempfile::tempdir().unwrap();
    let (client, paths, _envelopes) = asked(
        dir.path(),
        Some(validating()),
        ActionOutcome::Replied {
            body: r#"{"accept":true}"#.to_string(),
        },
    )
    .await;

    assert_eq!(
        ask_dog(
            &client,
            &paths,
            "kelpie",
            &table(&[("keep", serde_json::json!(2))]),
            None,
        )
        .await,
        Ok(())
    );
}

/// A dog that declared nothing is never asked, and never costs a round trip.
#[tokio::test]
async fn a_dog_without_validate_is_never_asked() {
    for manifest in [
        None,
        Some(r#"{"manifest":1,"name":"kelpie","version":"0.2.0","protocol":12,"channel":true}"#),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (client, paths, mut envelopes) =
            asked(dir.path(), manifest, ActionOutcome::TimedOut).await;

        assert_eq!(
            ask_dog(&client, &paths, "kelpie", &table(&[]), None).await,
            Ok(()),
            "{manifest:?}"
        );
        assert_eq!(
            envelopes.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty),
            "{manifest:?}: nothing was sent to the dog"
        );
    }
}

/// A dog an operator has not started yet is the ordinary case for a first
/// write, and nothing in the flock can refuse it.
#[tokio::test]
async fn a_dog_that_is_not_in_the_flock_is_not_asked() {
    let dir = tempfile::tempdir().unwrap();
    let paths = home_with(dir.path(), Some(validating()));
    let path = control_address(dir.path());
    let (client, _envelopes) = fake_client_answering_refusals(&path, |request| match request {
        Request::Trigger { .. } => Err(RpcError {
            code: RpcErrorCode::NotFound,
            message: "no sheep named kelpie".to_string(),
            daemon_version: None,
        }),
        _other => Ok(Response::Pong),
    })
    .await;

    assert_eq!(
        ask_dog(&client, &paths, "kelpie", &table(&[]), None).await,
        Ok(())
    );
}

/// A dog registered but not running, or one being swapped out, has no live
/// channel to answer on, and a write is what makes it worth starting.
#[tokio::test]
async fn a_dog_with_no_live_channel_is_not_asked() {
    for outcome in [
        ActionOutcome::DogNoChannel,
        ActionOutcome::NoChannel,
        ActionOutcome::Skipped,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (client, paths, _envelopes) =
            asked(dir.path(), Some(validating()), outcome.clone()).await;

        assert_eq!(
            ask_dog(&client, &paths, "kelpie", &table(&[]), None).await,
            Ok(()),
            "{outcome:?}"
        );
    }
}

/// An ask that never reached the shepherd is not the dog refusing.
#[tokio::test]
async fn an_ask_the_shepherd_refused_is_not_the_dog_refusing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = home_with(dir.path(), Some(validating()));
    let path = control_address(dir.path());
    let (client, _envelopes) = fake_client_answering_refusals(&path, |request| match request {
        Request::Trigger { .. } => Err(RpcError {
            code: RpcErrorCode::Forbidden,
            message: "dog `kelpie` was refused `trigger`".to_string(),
            daemon_version: None,
        }),
        _other => Ok(Response::Pong),
    })
    .await;

    let failure = ask_dog(&client, &paths, "kelpie", &table(&[]), None)
        .await
        .expect_err("the ask failed");

    assert!(matches!(failure, AskFailure::Transport(_)), "{failure:?}");
}
