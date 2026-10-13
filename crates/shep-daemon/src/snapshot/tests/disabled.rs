//! The disabled mark in the roll: written from the live listing, read back as
//! false when an older roll has no key, handed to the supervisor by a restore
//! before the app is registered at rest, and warned about when a restored
//! sheep depends on a disabled one.

use super::super::*;
use super::info;

use crate::fake::{ProcScript, ScriptedRunner};
use crate::supervisor::spawn_supervisor;
use crate::testing::{capture_logs, test_paths};
use shep_core::config::{AppConfig, normalize};
use shep_core::protocol::{BusEvent, ProcessEventKind};
use shep_core::status::ProcStatus;

fn saved(name: &str, instances_running: u32, disabled: bool) -> SavedApp {
    SavedApp {
        app: AppConfig::minimal(name, "./srv"),
        instances_running,
        disabled,
    }
}

/// An app that waits for `db`, saved as running.
fn saved_dependent(name: &str, dependency: &str) -> SavedApp {
    let mut app = AppConfig::minimal(name, "./srv");
    app.depends_on = vec![dependency.to_string()];
    SavedApp {
        app,
        instances_running: 1,
        disabled: false,
    }
}

/// Fails if `roll` stops reading the mark off the listing: a restart would
/// bring every parked sheep back.
#[test]
fn roll_carries_the_mark_of_a_disabled_app_and_no_other() {
    let registry = FlockRegistry::new();
    let apps = ["api", "web"].map(|name| normalize(AppConfig::minimal(name, "./srv")).unwrap());
    registry.record(&apps);

    let parked = ProcessInfo::builder(0, "api", ProcStatus::Stopped)
        .disabled(Some(true))
        .build();
    let roll = registry.roll(&[parked, info(1, "web", ProcStatus::Online)], 0);

    let marks: Vec<(&str, bool)> = roll
        .apps
        .iter()
        .map(|saved| (saved.app.name.as_str(), saved.disabled))
        .collect();
    assert_eq!(marks, [("api", true), ("web", false)]);
}

#[test]
fn the_mark_survives_the_roll_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("flock.json");
    let roll = FlockSnapshot::with_apps(vec![saved("api", 0, true), saved("web", 1, false)]);

    write_atomic(&path, &roll).unwrap();

    assert_eq!(read(&path).unwrap(), roll);
}

/// Fails if the field loses `#[serde(default)]`: a roll written by an older
/// shepherd would refuse to load and take the whole flock with it.
#[test]
fn a_roll_with_no_mark_reads_as_not_disabled() {
    let mut written = serde_json::to_value(FlockSnapshot::with_apps(vec![saved("web", 1, true)]))
        .expect("a roll serializes");
    written["apps"][0]
        .as_object_mut()
        .expect("an app entry is an object")
        .remove("disabled")
        .expect("the roll writes the key");

    let older: FlockSnapshot = serde_json::from_value(written).expect("an older roll loads");

    assert!(!older.apps[0].disabled);
}

/// A hand-edited roll can mark an app that was also recorded as up. The mark
/// wins: restoring it would be the spawn the mark exists to refuse.
#[test]
fn restorable_never_starts_a_disabled_app() {
    let restorable = restorable(FlockSnapshot::with_apps(vec![
        saved("api", 1, true),
        saved("web", 1, false),
    ]));

    let starting: Vec<&str> = restorable
        .to_start
        .iter()
        .map(|app| app.config().name.as_str())
        .collect();
    assert_eq!(starting, ["web"]);
    assert_eq!(
        restorable.members.len(),
        2,
        "a parked sheep is still a member"
    );
    assert_eq!(
        restorable
            .disabled
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["api"]
    );
}

/// Fails if `muster` stops handing the marked names to the supervisor before
/// it registers them: `api` would come back listed, stopped and enabled.
#[tokio::test(start_paused = true)]
async fn a_restore_keeps_a_disabled_sheep_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    std::fs::create_dir_all(paths.snapshot.parent().unwrap()).unwrap();
    let roll = FlockSnapshot::with_apps(vec![saved("api", 0, true), saved("web", 1, false)]);
    write_atomic(&paths.snapshot, &roll).unwrap();

    let (events, _rx) = crate::bus::test_bus(64);
    let handle = spawn_supervisor(
        ScriptedRunner::new(vec![ProcScript::never_exits()]),
        paths.clone(),
        events.clone(),
    );

    muster(
        &paths.snapshot,
        &FlockRegistry::new(),
        &handle,
        &events,
        &[],
        &[],
    )
    .await
    .unwrap();

    let mut listed = handle.list_checked().await.unwrap();
    listed.sort_by(|left, right| left.name.cmp(&right.name));
    let seen: Vec<(&str, ProcStatus, Option<bool>)> = listed
        .iter()
        .map(|info| (info.name.as_str(), info.status, info.disabled))
        .collect();
    assert_eq!(
        seen,
        [
            ("api", ProcStatus::Stopped, Some(true)),
            ("web", ProcStatus::Online, None),
        ]
    );
    handle.shutdown().await;
}

/// Fails if a disabled dependency keeps its dependent down, or if the roll
/// round trip through the next write drops the mark again.
#[tokio::test(start_paused = true)]
async fn a_dependent_of_a_disabled_sheep_still_starts() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    std::fs::create_dir_all(paths.snapshot.parent().unwrap()).unwrap();
    let roll = FlockSnapshot::with_apps(vec![saved("db", 0, true), saved_dependent("api", "db")]);
    write_atomic(&paths.snapshot, &roll).unwrap();

    let (events, _rx) = crate::bus::test_bus(64);
    let handle = spawn_supervisor(
        ScriptedRunner::new(vec![ProcScript::never_exits()]),
        paths.clone(),
        events.clone(),
    );
    let mut seen = events.subscribe();

    muster(
        &paths.snapshot,
        &FlockRegistry::new(),
        &handle,
        &events,
        &[],
        &[],
    )
    .await
    .unwrap();

    let mut started = Vec::new();
    while let Ok(event) = seen.try_recv() {
        if let BusEvent::Process {
            event: ProcessEventKind::Start,
            info,
            ..
        } = &*event
        {
            started.push(info.name.clone());
        }
    }
    assert_eq!(started, ["api"]);
    handle.shutdown().await;
}

#[test]
fn a_disabled_dependency_is_warned_about_beside_the_autostart_one() {
    let mut api = AppConfig::minimal("api", "./srv");
    api.depends_on = vec!["db".to_string(), "cache".to_string()];
    let to_start = vec![normalize(api).unwrap()];
    let disabled = BTreeSet::from(["db".to_string()]);

    let logged = capture_logs(|| warn_about_disabled_dependencies(&to_start, &disabled));

    assert!(
        logged.contains("the dependency is disabled; starting without it"),
        "{logged}"
    );
    assert!(logged.contains("db"), "{logged}");
    assert!(!logged.contains("cache"), "{logged}");
}
