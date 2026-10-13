//! A saved sheep keeps its name when a dog wants it
//!
//! Listing the built-in dogs at rest must never cost anyone a sheep, so on
//! an upgrade a roll's sheep named `metrics` stays, whatever the dog's
//! standing, and boot says which of the two to rename.
//!
//! `#[test]` with a `block_on` of its own, not `#[tokio::test]`:
//! `capture_logs` scopes its subscriber to a synchronous closure.

use crate::boot::*;
use crate::dogs::DogSpec;
use crate::fake::{ProcScript, ScriptedRunner};
use crate::testing::{capture_logs, roll_of, test_paths};
use shep_core::config::AppConfig;
use shep_core::protocol::{DogSource, ProcessInfo};

fn built_in(name: &str) -> DogSpec {
    DogSpec {
        name: name.to_string(),
        source: DogSource::BuiltIn,
    }
}

/// Boots over a roll holding one sheep named `metrics`, and returns the
/// flock with everything the boot logged.
fn boot_over_a_saved_metrics(
    scripts: Vec<ProcScript>,
    options: BootOptions,
) -> (Vec<ProcessInfo>, String) {
    let _guard = SIGNAL_TEST_LOCK.blocking_lock();
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    init_dirs(&paths).unwrap();
    let roll = roll_of(vec![AppConfig::minimal("metrics", "./srv")]);
    crate::snapshot::write_atomic(&paths.snapshot, &roll).unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let mut boot_result = None;
    let logs = capture_logs(|| {
        boot_result = Some(rt.block_on(boot_with_idle_sweep(
            ScriptedRunner::new(scripts),
            paths.clone(),
            BootOptions {
                restore: true,
                ..options
            },
        )));
    });
    let daemon = boot_result
        .unwrap()
        .expect("a name collision must not fail the boot");
    let flock = rt
        .block_on(daemon.context().supervisor.list_checked())
        .unwrap();
    drop(daemon);
    (flock, logs)
}

#[test]
fn a_saved_sheep_named_after_a_listed_built_in_survives_and_the_built_in_is_not_registered() {
    let (flock, logs) = boot_over_a_saved_metrics(
        vec![ProcScript::never_exits()],
        BootOptions {
            known_dogs: vec![built_in("metrics"), built_in("bark")],
            ..BootOptions::default()
        },
    );

    let held: Vec<&ProcessInfo> = flock.iter().filter(|info| info.name == "metrics").collect();
    assert_eq!(held.len(), 1, "one name is one entry: {flock:?}");
    assert!(held[0].dog.is_none(), "the sheep keeps the name: {flock:?}");
    assert!(
        flock
            .iter()
            .any(|info| info.name == "bark" && info.dog.is_some()),
        "the other built-in is still listed: {flock:?}"
    );
    assert!(
        logs.contains("WARN") && logs.contains("rename"),
        "the warning must say to rename one: {logs:?}"
    );
}

#[test]
fn a_saved_sheep_named_after_an_enabled_built_in_survives_and_the_built_in_does_not_start() {
    let (flock, logs) = boot_over_a_saved_metrics(
        // One script: the sheep's. The dog must not consume another.
        vec![ProcScript::never_exits()],
        BootOptions {
            dogs: vec![built_in("metrics")],
            known_dogs: vec![built_in("metrics")],
            ..BootOptions::default()
        },
    );

    assert_eq!(flock.len(), 1, "{flock:?}");
    assert!(flock[0].dog.is_none(), "{flock:?}");
    assert!(logs.contains("rename"), "{logs:?}");
}

#[test]
fn a_boot_first_dog_colliding_with_a_saved_sheep_keeps_the_sheep() {
    let (flock, logs) = boot_over_a_saved_metrics(
        // One script: the sheep's. The promoted dog must not take it.
        vec![ProcScript::never_exits()],
        BootOptions {
            dogs: vec![built_in("metrics")],
            known_dogs: vec![built_in("metrics")],
            boot_first_dogs: vec!["metrics".to_string()],
            ..BootOptions::default()
        },
    );

    assert_eq!(flock.len(), 1, "one name is one entry: {flock:?}");
    assert!(
        flock[0].dog.is_none(),
        "the sheep holds the name: {flock:?}"
    );
    assert_eq!(
        flock[0].status,
        shep_core::status::ProcStatus::Online,
        "the sheep was running when the roll was written: {flock:?}"
    );
    assert!(logs.contains("rename"), "{logs:?}");
}
