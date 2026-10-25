//! `CopySheepEnv`: one sheep's env onto another's inside the shepherd, so
//! no value reaches the client.

use shep_core::protocol::{EnvCopyChange, EnvCopyKind};

use super::*;

/// Starts `web` and `api` with the env each case below copies between.
///
/// `web` holds a reference, two literals and a key `api` shares; `api` holds
/// the shared key, a different `B`, and `D`, which only it has.
async fn start_pair(h: &Harness) {
    shep_core::secrets::set(
        &h.ctx.paths.secrets,
        "PADDOCK_KEY",
        "production",
        "paddock-value",
    )
    .unwrap();
    let mut web = AppConfig::minimal("web", "./srv");
    for (key, value) in [
        ("PADDOCK_KEY", "{{secret:PADDOCK_KEY}}"),
        ("B", "web-literal"),
        ("C", "same"),
    ] {
        web.env.insert(key.to_string(), value.to_string());
    }
    let mut api = AppConfig::minimal("api", "./srv");
    for (key, value) in [("B", "api-literal"), ("C", "same"), ("D", "only-api")] {
        api.env.insert(key.to_string(), value.to_string());
    }
    let started = reply_of(
        dispatch(
            envelope(
                1,
                Request::Start {
                    apps: vec![web, api],
                },
            ),
            &h.ctx,
        )
        .await,
    );
    assert!(started.result.is_ok(), "{:?}", started.result);
}

/// Sends one `CopySheepEnv` web onto api.
async fn copy(
    h: &Harness,
    id: u64,
    replace: bool,
    except: &[&str],
    dry_run: bool,
) -> Result<Response, RpcError> {
    copy_expecting(h, id, replace, except, dry_run, None).await
}

/// Sends one `CopySheepEnv` web onto api, holding the shepherd to the change
/// list `expect` names.
async fn copy_expecting(
    h: &Harness,
    id: u64,
    replace: bool,
    except: &[&str],
    dry_run: bool,
    expect: Option<Vec<EnvCopyChange>>,
) -> Result<Response, RpcError> {
    reply_of(
        dispatch(
            envelope(
                id,
                Request::CopySheepEnv {
                    from: "web".to_string(),
                    to: "api".to_string(),
                    replace,
                    except: except.iter().map(|key| (*key).to_string()).collect(),
                    dry_run,
                    expect,
                },
            ),
            &h.ctx,
        )
        .await,
    )
    .result
}

fn changes_of(reply: Result<Response, RpcError>) -> Vec<EnvCopyChange> {
    match reply {
        Ok(Response::SheepEnvCopied { changes, .. }) => changes,
        other => panic!("expected SheepEnvCopied, got {other:?}"),
    }
}

/// `changes` as the shepherd lists them with the seals taken off, for
/// comparing against what a test writes out by hand.
fn unsealed(changes: Vec<EnvCopyChange>) -> Vec<EnvCopyChange> {
    changes.into_iter().map(|change| change.sealed(0)).collect()
}

fn stored_env(h: &Harness, name: &str) -> serde_json::Value {
    shep_core::overrides::get(&h.ctx.paths.overrides, name)
        .unwrap()
        .map_or(serde_json::Value::Null, |record| {
            record
                .fields
                .get("env")
                .cloned()
                .unwrap_or(serde_json::Value::Null)
        })
}

/// A merge adds and overwrites, leaves the target's own key, writes the
/// reference as the reference, and tells the client nothing but that.
#[tokio::test(start_paused = true)]
async fn a_merge_copies_every_key_and_shows_only_references() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;

    let reply = copy(&h, 2, false, &[], false).await;
    let wire = format!("{reply:?}");
    for literal in ["web-literal", "api-literal", "only-api"] {
        assert!(!wire.contains(literal), "{literal} leaked: {wire}");
    }
    assert_eq!(
        unsealed(changes_of(reply)),
        vec![
            EnvCopyChange::new(
                "B",
                EnvCopyKind::Changed,
                Some("<redacted>".into()),
                Some("<redacted>".into())
            ),
            EnvCopyChange::new(
                "PADDOCK_KEY",
                EnvCopyKind::Added,
                None,
                Some("{{secret:PADDOCK_KEY}}".into())
            ),
        ]
    );

    let stored = stored_env(&h, "api");
    assert_eq!(stored["PADDOCK_KEY"], "{{secret:PADDOCK_KEY}}");
    assert_eq!(stored["B"], "web-literal");
    assert!(
        stored.get("D").is_none(),
        "a merge leaves D alone: {stored}"
    );

    let api = describe(&h.ctx, 3, SelectorSpec::Name("api".to_string())).await;
    assert_eq!(
        api[0].pending.as_deref(),
        Some(["env".to_string()].as_slice()),
        "env parks for the next spawn"
    );
}

/// `replace` makes the target's env exactly the source's.
#[tokio::test(start_paused = true)]
async fn replace_removes_the_keys_only_the_target_had() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;

    let changes = changes_of(copy(&h, 2, true, &[], false).await);
    let kinds: Vec<(&str, EnvCopyKind)> = changes
        .iter()
        .map(|change| (change.key.as_str(), change.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("B", EnvCopyKind::Changed),
            ("D", EnvCopyKind::Removed),
            ("PADDOCK_KEY", EnvCopyKind::Added),
        ]
    );
    assert_eq!(stored_env(&h, "api")["D"], serde_json::Value::Null);
}

/// An excepted key stays as the target had it, under a merge and under
/// `replace`, and one the target lacks is not brought over.
#[tokio::test(start_paused = true)]
async fn an_excepted_key_is_left_as_the_target_had_it() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;

    let changes = changes_of(copy(&h, 2, true, &["B", "D", "PADDOCK_KEY"], false).await);
    assert!(changes.is_empty(), "{changes:?}");
    assert_eq!(stored_env(&h, "api"), serde_json::Value::Null);
}

/// A dry run answers what a real run would, and writes nothing.
#[tokio::test(start_paused = true)]
async fn a_dry_run_writes_nothing() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;

    let dry = changes_of(copy(&h, 2, false, &[], true).await);
    assert_eq!(dry.len(), 2);
    assert_eq!(stored_env(&h, "api"), serde_json::Value::Null);
    let api = describe(&h.ctx, 3, SelectorSpec::Name("api".to_string())).await;
    assert_eq!(api[0].pending, None);

    let real = changes_of(copy(&h, 4, false, &[], false).await);
    assert_eq!(real, dry);
}

/// Copying twice finds nothing left to change.
#[tokio::test(start_paused = true)]
async fn a_second_copy_changes_nothing() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;
    changes_of(copy(&h, 2, false, &[], false).await);
    assert!(changes_of(copy(&h, 3, false, &[], false).await).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_missing_sheep_on_either_side_is_not_found() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;
    for (id, from, to) in [(2, "ghost", "api"), (3, "web", "ghost")] {
        let reply = reply_of(
            dispatch(
                envelope(
                    id,
                    Request::CopySheepEnv {
                        from: from.to_string(),
                        to: to.to_string(),
                        replace: false,
                        except: Vec::new(),
                        dry_run: false,
                        expect: None,
                    },
                ),
                &h.ctx,
            )
            .await,
        );
        let Err(err) = reply.result else {
            panic!("{from} to {to} must be refused");
        };
        assert_eq!(err.code, RpcErrorCode::NotFound, "{err:?}");
        assert!(err.message.contains("ghost"), "{err:?}");
    }
}

/// A dog's env is not an operator's to edit, in either direction.
#[tokio::test(start_paused = true)]
async fn a_dog_on_either_side_is_refused() {
    let h = harness(vec![
        ProcScript::never_exits(),
        ProcScript::never_exits(),
        ProcScript::never_exits(),
    ]);
    start_pair(&h).await;
    enable_dog(&h.ctx, 2, "bark").await;
    for (id, from, to) in [(3, "web", "bark"), (4, "bark", "web")] {
        let reply = reply_of(
            dispatch(
                envelope(
                    id,
                    Request::CopySheepEnv {
                        from: from.to_string(),
                        to: to.to_string(),
                        replace: false,
                        except: Vec::new(),
                        dry_run: false,
                        expect: None,
                    },
                ),
                &h.ctx,
            )
            .await,
        );
        let Err(err) = reply.result else {
            panic!("{from} to {to} must be refused");
        };
        assert_eq!(err.code, RpcErrorCode::InvalidConfig, "{err:?}");
    }
}

/// The write is held to the change list the operator reviewed: a key another
/// client added to the target since then would be deleted by `replace` without
/// ever having been shown, so the shepherd refuses and writes nothing.
#[tokio::test(start_paused = true)]
async fn a_write_whose_reviewed_changes_no_longer_hold_is_refused() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;
    let reviewed = changes_of(copy(&h, 2, true, &[], true).await);

    let edited = reply_of(
        dispatch(
            envelope(
                3,
                Request::SetSheepEnv {
                    name: "api".to_string(),
                    key: "ADDED_SINCE".to_string(),
                    value: Some("late".to_string().into()),
                },
            ),
            &h.ctx,
        )
        .await,
    );
    assert!(edited.result.is_ok(), "{:?}", edited.result);
    let before = stored_env(&h, "api");

    let err = copy_expecting(&h, 4, true, &[], false, Some(reviewed))
        .await
        .unwrap_err();
    assert_eq!(err.code, RpcErrorCode::InvalidConfig, "{err:?}");
    assert!(err.message.contains("changed"), "{err:?}");
    assert_eq!(stored_env(&h, "api"), before, "nothing was written");
}

/// A change list that still holds is applied.
#[tokio::test(start_paused = true)]
async fn a_write_whose_reviewed_changes_still_hold_is_applied() {
    let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
    start_pair(&h).await;
    let reviewed = changes_of(copy(&h, 2, false, &[], true).await);

    let applied =
        changes_of(copy_expecting(&h, 3, false, &[], false, Some(reviewed.clone())).await);
    assert_eq!(applied, reviewed);
    assert_eq!(stored_env(&h, "api")["B"], "web-literal");
}

/// Two redacted values look alike, so the seal is what notices a literal
/// swapped for another, on the target or on the source, between the dry run
/// and the write.
#[tokio::test(start_paused = true)]
async fn a_literal_swapped_since_the_review_is_refused() {
    for (id, edited) in [(3, "api"), (5, "web")] {
        let h = harness(vec![ProcScript::never_exits(), ProcScript::never_exits()]);
        start_pair(&h).await;
        let reviewed = changes_of(copy(&h, 2, false, &[], true).await);

        let swapped = reply_of(
            dispatch(
                envelope(
                    id,
                    Request::SetSheepEnv {
                        name: edited.to_string(),
                        key: "B".to_string(),
                        value: Some("swapped".to_string().into()),
                    },
                ),
                &h.ctx,
            )
            .await,
        );
        assert!(swapped.result.is_ok(), "{:?}", swapped.result);
        let before = stored_env(&h, "api");

        let err = copy_expecting(&h, id + 1, false, &[], false, Some(reviewed))
            .await
            .unwrap_err();
        assert_eq!(err.code, RpcErrorCode::InvalidConfig, "{edited}: {err:?}");
        assert!(err.message.contains('B'), "{edited}: {err:?}");
        assert_eq!(stored_env(&h, "api"), before, "nothing was written");
    }
}
