//! How a disabled app reads in lookout's STATUS cells, group and fold
//! headers included, and that they agree with `shep flock`'s words.

use super::testing::started;
use super::*;

fn disabled_web(id: u32, slot: u32) -> ProcessInfo {
    ProcessInfo::builder(id, "web", ProcStatus::Stopped)
        .instance(Some(slot))
        .fold(Some("front".to_string()))
        .disabled(Some(true))
        .build()
}

fn app_with(rows: Vec<ProcessInfo>) -> App {
    let (mut app, t0) = started();
    app.update(Msg::Snapshot { rows, at: t0 });
    app
}

#[test]
fn a_disabled_sheep_row_reads_disabled() {
    let app = app_with(vec![disabled_web(1, 0)]);
    let rows = app.rows();

    assert_eq!(rows[0].reported().word(), "disabled");
}

/// fails if the rollup reads `ProcStatus` alone: the leaf rows would say
/// `disabled` and the header above them `stopped`.
#[test]
fn a_group_of_disabled_instances_reads_disabled() {
    let rows = vec![disabled_web(1, 0), disabled_web(2, 1)];
    let app = app_with(rows);

    assert_eq!(app.group_status_text("web"), "disabled");
}

#[test]
fn a_fold_of_disabled_instances_reads_disabled() {
    let app = app_with(vec![disabled_web(1, 0), disabled_web(2, 1)]);

    assert_eq!(app.fold_status_text("front"), "disabled");
}

#[test]
fn a_mixed_group_counts_disabled_apart_from_stopped() {
    let mut rows = vec![disabled_web(1, 0), disabled_web(2, 1)];
    rows[1] = ProcessInfo::builder(2, "web", ProcStatus::Stopped)
        .instance(Some(1))
        .fold(Some("front".to_string()))
        .build();
    let app = app_with(rows);

    assert_eq!(app.group_status_text("web"), "1 disabled, 1 stopped");
}
