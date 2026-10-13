//! Dogs across a handover: which ones the blob carries, and what a successor
//! does with the ones its own `shep.toml` does not enable.
//!
//! The predecessor never reads `shep.toml`, so a dog's mark is its absence
//! from the enabled list the caller handed it. That list is the successor's to
//! read, which is why a parked dog is left behind rather than carried.

use super::*;

/// One dog as the boot hands it over: built-in, and named as `shep.toml`
/// would name it.
fn dog(name: &str) -> crate::dogs::DogSpec {
    crate::dogs::DogSpec {
        name: name.to_string(),
        source: DogSource::BuiltIn,
    }
}

/// The app [`dog`] names, assembled the way the boot assembles it.
fn dog_app(name: &str, paths: &shep_core::paths::ShepPaths) -> shep_core::config::ResolvedApp {
    crate::dogs::dog_app(&dog(name), paths).expect("a built-in dog assembles")
}

fn names(wanted: &[&str]) -> BTreeSet<String> {
    wanted.iter().map(|name| (*name).to_string()).collect()
}

/// The blob describes the flock this daemon supervises, minus the dogs its
/// caller's config leaves out. A dog an operator stopped with `shep stop`
/// carries, since the config still names it: only the mark decides.
#[cfg(unix)]
#[tokio::test(start_paused = true)]
async fn a_snapshot_carries_every_dog_that_is_not_disabled_and_leaves_one_that_is() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let (events, _rx) = crate::bus::test_bus(64);
    let (fds, _held) = daemon_fds(&dir);
    let runner = ScriptedRunner::new(vec![ProcScript::never_exits(); 3]);
    let sup = spawn_supervisor(runner, paths.clone(), events);
    sup.start(vec![normalize(AppConfig::minimal("api", "./srv")).unwrap()])
        .await
        .unwrap();
    // Running, and enabled: the ordinary carried dog.
    sup.start_dog(dog_app("metrics", &paths), DogSource::BuiltIn)
        .await
        .unwrap();
    // `Stopped` with no mark, which is the shape `shep stop` leaves.
    sup.register_dogs_at_rest(
        vec![(dog_app("bark", &paths), DogSource::BuiltIn)],
        BTreeSet::new(),
    )
    .await
    .unwrap();
    // Disabled: absent from the enabled list, which is a dog's whole mark.
    sup.register_dogs_at_rest(
        vec![(
            dog_app("log-rotate", &paths),
            DogSource::Adopted {
                path: "/opt/bin/shep-log-rotate".to_string(),
                channel: false,
            },
        )],
        names(&["log-rotate"]),
    )
    .await
    .unwrap();

    let (_candidates, blob, _parked) = sup.handover_snapshot(fds).await.unwrap();

    let carried: Vec<(&str, Option<DogSource>)> = blob
        .sheep()
        .iter()
        .map(|sheep| (sheep.name(), sheep.dog().cloned()))
        .collect();
    assert_eq!(
        carried,
        [
            ("api", None),
            ("metrics", Some(DogSource::BuiltIn)),
            ("bark", Some(DogSource::BuiltIn)),
        ],
        "a parked dog belongs to the successor's file, not to this flock's blob"
    );
}

/// The successor's half. `metrics` crossed the `execve` running, because the
/// predecessor booted while the file named it, and the file this successor
/// boots from no longer does: it goes down through the same door `shep disable`
/// uses, so it ends stopped, marked, and with `shep enable <name>` as the way
/// back. `bark` was never carried, and lands at rest under the id this
/// successor issues.
#[cfg(unix)]
#[tokio::test]
async fn a_successor_stops_a_carried_dog_its_config_no_longer_enables() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let (events, _rx) = crate::bus::test_bus(64);
    let pid = adoptable_child("sleep 30");
    let sup = SupervisorBuilder::new(TokioRunner::new(), paths.clone(), events)
        .spawn_adopted(
            vec![
                without_handles(carried("api", 0, None, |entry| {
                    entry.status = ProcStatus::Stopped;
                })),
                without_handles(carried("metrics", 1, Some(pid), |entry| {
                    entry.dog = Some(DogSource::BuiltIn);
                })),
            ],
            counters(2),
            Vec::new(),
        )
        .expect("a carried flock installs");

    crate::dogs::register_dogs_at_rest(&[dog("metrics"), dog("bark")], &[], &paths, &sup).await;

    let rows = flock_until(
        &sup,
        |rows| {
            rows.iter()
                .any(|row| row.name == "metrics" && row.status == ProcStatus::Stopped)
        },
        "a dog the successor's config does not enable must stop, not just be listed",
    )
    .await;

    let metrics = rows.iter().find(|row| row.name == "metrics").unwrap();
    assert_eq!(metrics.id, 1, "the carried row is the successor's to stop");
    assert_eq!(metrics.pid, None, "{metrics:?}");
    assert_eq!(metrics.disabled, Some(true), "{metrics:?}");
    assert_eq!(
        metrics.last_exit,
        Some(ExitInfo {
            code: None,
            signal: Some(15),
        }),
        "the process that crossed the exec must have been signalled, not dropped from the flock"
    );

    let bark = rows.iter().find(|row| row.name == "bark").unwrap();
    assert_eq!(bark.status, ProcStatus::Stopped, "{bark:?}");
    assert_eq!(bark.disabled, Some(true), "{bark:?}");
    assert_eq!(
        bark.id, 2,
        "a dog left behind gets the id this successor issues"
    );
    assert_eq!(
        rows.iter().find(|row| row.name == "api").unwrap().id,
        0,
        "the flock the successor inherited keeps its rows"
    );
}

/// The other half of the same file read: a dog the successor's config still
/// enables keeps its carried row, its pid and its process. Stopping it here
/// would turn every reload into a restart of the monitoring.
#[cfg(unix)]
#[tokio::test]
async fn a_successor_leaves_a_carried_dog_its_config_still_enables_running() {
    let dir = tempfile::tempdir().unwrap();
    let paths = test_paths(&dir);
    let (events, _rx) = crate::bus::test_bus(64);
    let pid = adoptable_child("sleep 30");
    let sup = SupervisorBuilder::new(TokioRunner::new(), paths.clone(), events)
        .spawn_adopted(
            vec![without_handles(carried("metrics", 1, Some(pid), |entry| {
                entry.dog = Some(DogSource::BuiltIn);
            }))],
            counters(2),
            Vec::new(),
        )
        .expect("a carried flock installs");

    crate::dogs::register_dogs_at_rest(&[dog("metrics")], &[dog("metrics")], &paths, &sup).await;

    let rows = sup.list().await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    let metrics = &rows[0];
    assert_eq!(metrics.status, ProcStatus::Online, "{metrics:?}");
    assert_eq!(
        metrics.pid,
        Some(pid),
        "a dog the config enables must keep the process it carried"
    );
    assert_eq!(metrics.disabled, None, "{metrics:?}");
}
