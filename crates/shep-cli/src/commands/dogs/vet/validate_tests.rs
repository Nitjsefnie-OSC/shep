//! `shep adopt` against a manifest that asks to be consulted.
//!
//! `validate` is a promise about a check shep will run, and the check runs on
//! the dog's channel. A manifest that promises the check without the channel
//! is refused at adopt, before `shep.toml` says the dog is adopted.

use std::path::{Path, PathBuf};
use std::time::Duration;

use shep_core::paths::ShepPaths;

use super::*;
use crate::cli::AdoptArgs;
use crate::commands::dogs::adopt;

/// A budget for tests that are not about the budget: the probe spawns a real
/// child and bounds its wait on a wall clock.
const BUDGET: Duration = Duration::from_secs(30);

fn streams<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>) -> Streams<'a> {
    Streams {
        out,
        err,
        style: crate::style::Presentation::BARE,
        fmt: crate::cli::Format::Table,
    }
}

/// A dog that answers [`MANIFEST_FLAG`](shep_core::dogs::MANIFEST_FLAG) with
/// `manifest` and nothing to the other two probes.
fn manifest_dog(dir: &Path, name: &str, manifest: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\ncase \"$1\" in\n{flag})\necho '{manifest}'\n;;\nesac\n",
            flag = shep_core::dogs::MANIFEST_FLAG
        ),
    )
    .unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

/// The four fields a manifest has to answer with, plus the two this is about.
fn declared(channel: bool, validate: bool) -> String {
    serde_json::json!({
        "manifest": 1,
        "name": "kelpie",
        "version": "0.2.0",
        "protocol": shep_core::protocol::PROTOCOL_VERSION,
        "channel": channel,
        "validate": validate,
    })
    .to_string()
}

/// The whole refusal, at the vet and at the verb, with nothing written.
#[test]
fn a_dog_that_asks_to_be_consulted_without_a_channel_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let bin = manifest_dog(dir.path(), "shep-kelpie", &declared(false, true));

    assert_eq!(
        vet_binary_within(&bin, dir.path(), "kelpie", BUDGET),
        Err(AdoptRefusal::ValidateWithoutChannel)
    );
}

/// The operator reads why, and what to change.
#[tokio::test]
async fn adopt_refuses_that_manifest_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    let bin = manifest_dog(dir.path(), "shep-kelpie", &declared(false, true));
    let args = AdoptArgs {
        name: Some("kelpie".to_string()),
        path: bin,
    };

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = adopt(&mut streams(&mut out, &mut err), &paths, &args).await;

    let text = String::from_utf8(err).unwrap();
    assert_eq!(code, ExitCode::InvalidConfig, "{text}");
    assert!(
        text.contains("consulted") && text.contains("`channel`") && text.contains("`validate`"),
        "the refusal names both fields it needs: {text}"
    );
    assert!(
        !paths.daemon_config.exists(),
        "a refused adopt writes no `shep.toml`"
    );
}

/// The same manifest with the channel it needs is an ordinary adopt.
#[tokio::test]
async fn the_same_manifest_with_a_channel_is_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    let bin = manifest_dog(dir.path(), "shep-kelpie", &declared(true, true));
    let args = AdoptArgs {
        name: Some("kelpie".to_string()),
        path: bin.clone(),
    };

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = adopt(&mut streams(&mut out, &mut err), &paths, &args).await;

    assert_eq!(
        code,
        ExitCode::Success,
        "{}",
        String::from_utf8(err).unwrap()
    );
    assert!(
        vet_binary_within(&bin, dir.path(), "kelpie", BUDGET)
            .expect("adopted")
            .manifest
            .expect("a manifest")
            .validate,
        "the declaration survives to the cache `shep set` reads"
    );
}
