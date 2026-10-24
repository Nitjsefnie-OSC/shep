//! `shep dogs grant` on a built-in dog: its own manifest is what it declares.

use shep_core::paths::ShepPaths;
use shep_core::permissions::Grants;

use super::grant::{Change, change};
use super::test_support::streams;
use crate::commands::shep_toml::{GrantRecord, ShepToml};
use crate::exit::ExitCode;

#[tokio::test]
async fn a_built_in_dogs_waiting_permission_is_approved_by_a_grant() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    std::fs::create_dir_all(&paths.home).unwrap();
    std::fs::write(&paths.daemon_config, "").unwrap();
    ShepToml::try_edit(&paths.daemon_config, |cfg| {
        cfg.set_grant_record(
            "bark",
            &GrantRecord::reviewed(Grants::parse(["flock.read"]).unwrap()),
        )
    })
    .unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();

    let code = change(
        &mut streams(&mut out, &mut err),
        &paths,
        Change::Grant,
        "bark",
        "events:process.",
    )
    .await;

    let said = String::from_utf8(err).unwrap();
    assert_eq!(code, ExitCode::Success, "{said}");
    let record = ShepToml::read_only(&paths.daemon_config)
        .unwrap()
        .grant_record("bark")
        .unwrap()
        .expect("bark keeps its record");
    assert!(
        record.granted.names().any(|held| held == "events:process."),
        "{record:?}"
    );
    assert!(
        record
            .waiting
            .names()
            .all(|waits| waits != "events:process."),
        "{record:?}"
    );
}

#[tokio::test]
async fn a_built_in_dog_is_not_granted_what_it_does_not_declare() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    std::fs::create_dir_all(&paths.home).unwrap();
    std::fs::write(&paths.daemon_config, "").unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();

    let code = change(
        &mut streams(&mut out, &mut err),
        &paths,
        Change::Grant,
        "bark",
        "shepherd.control",
    )
    .await;

    let said = String::from_utf8(err).unwrap();
    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(said.contains("shepherd.control"), "{said}");
}
