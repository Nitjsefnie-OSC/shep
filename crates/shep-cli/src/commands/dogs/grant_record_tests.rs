//! `shep dogs grant` and `revoke` on a dog's upgrade record: what waits, what
//! was revoked, and the not-reviewed mark.

use std::path::{Path, PathBuf};

use shep_core::paths::ShepPaths;
use shep_core::permissions::Grants;

use super::grant::{Change, change};
use super::test_support::streams;
use crate::commands::shep_toml::{GrantRecord, ShepToml};
use crate::exit::ExitCode;

const DECLARES_TWO: &str = r#"{"manifest":1,"name":"otel","version":"0.2.0","protocol":12,"permissions":[{"name":"flock.read","required":true,"reason":"reads the flock to report on it"},{"name":"events:process.","required":false,"reason":"hears when a sheep exits"}]}"#;

fn write_dog(dir: &Path, manifest: &str) -> PathBuf {
    let binary = dir.join("shep-otel");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = --shep-manifest ]; then cat <<'EOF'\n{manifest}\nEOF\nfi\nexit 0\n"
    );
    std::fs::write(&binary, script).unwrap();
    let mut mode = std::fs::metadata(&binary).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&binary, mode).unwrap();
    binary
}

fn grants(names: &[&str]) -> Grants {
    Grants::parse(names).unwrap()
}

/// A home with `otel` adopted, disabled, and holding `record`.
fn home_holding(record: &GrantRecord) -> (tempfile::TempDir, ShepPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    let binary = write_dog(dir.path(), DECLARES_TWO);
    std::fs::create_dir_all(&paths.home).unwrap();
    std::fs::write(
        &paths.daemon_config,
        format!("[daemon.adopted_dogs]\notel = \"{}\"\n", binary.display()),
    )
    .unwrap();
    ShepToml::try_edit(&paths.daemon_config, |cfg| {
        cfg.set_grant_record("otel", record)
    })
    .unwrap();
    (dir, paths)
}

fn recorded(paths: &ShepPaths) -> GrantRecord {
    ShepToml::read_only(&paths.daemon_config)
        .unwrap()
        .grant_record("otel")
        .unwrap()
        .expect("otel has a record")
}

async fn run(verb: Change, paths: &ShepPaths, permission: &str) -> (ExitCode, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = change(
        &mut streams(&mut out, &mut err),
        paths,
        verb,
        "otel",
        permission,
    )
    .await;
    (code, String::from_utf8(err).unwrap())
}

#[tokio::test]
async fn a_grant_takes_the_permission_out_of_waiting_and_clears_the_mark() {
    let (_dir, paths) = home_holding(&GrantRecord::at_upgrade(grants(&["flock.read"])));
    // The build declares one more than the record holds, so it is waiting by
    // the time the grant is judged.

    let (code, said) = run(Change::Grant, &paths, "events:process.").await;

    assert_eq!(code, ExitCode::Success, "{said}");
    assert_eq!(
        recorded(&paths),
        GrantRecord::reviewed(grants(&["flock.read", "events:process."]))
    );
}

#[tokio::test]
async fn a_revoke_is_remembered_so_the_build_that_still_declares_it_is_not_asking_anew() {
    let (_dir, paths) = home_holding(&GrantRecord::reviewed(grants(&[
        "flock.read",
        "events:process.",
    ])));

    let (code, said) = run(Change::Revoke, &paths, "events:process.").await;

    assert_eq!(code, ExitCode::Success, "{said}");
    let expected = GrantRecord {
        revoked: grants(&["events:process."]),
        ..GrantRecord::reviewed(grants(&["flock.read"]))
    };
    assert_eq!(recorded(&paths), expected);

    let (code, said) = run(Change::Revoke, &paths, "flock.read").await;
    assert_eq!(code, ExitCode::Success, "{said}");
    assert!(
        recorded(&paths).waiting.names().next().is_none(),
        "a revoked permission is never waiting"
    );
}

#[tokio::test]
async fn a_grant_says_what_still_waits() {
    let (_dir, paths) = home_holding(&GrantRecord::reviewed(grants(&[])));

    let (code, said) = run(Change::Grant, &paths, "flock.read").await;

    assert_eq!(code, ExitCode::Success, "{said}");
    assert!(said.contains("still waits on events:process."), "{said}");
    assert_eq!(
        recorded(&paths).waiting,
        grants(&["events:process."]),
        "the other permission is not granted by naming one"
    );
}
