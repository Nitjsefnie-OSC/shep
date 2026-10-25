//! How a disabled sheep reads in `shep flock`: `disabled` in STATUS, and in
//! JSON the mark beside an unchanged `status`.

use shep_core::protocol::ProcessInfo;
use shep_core::status::ProcStatus;

use super::*;
use crate::output::Render;

fn status_cell(info: ProcessInfo) -> String {
    let headers = FlockRows::headers();
    let at = headers.iter().position(|h| *h == "STATUS").unwrap();
    FlockRows(vec![info]).rows()[0][at].clone()
}

fn api(status: ProcStatus, disabled: Option<bool>) -> ProcessInfo {
    ProcessInfo::builder(0, "api", status)
        .disabled(disabled)
        .build()
}

#[test]
fn a_stopped_sheep_whose_app_is_disabled_reads_disabled_in_status() {
    assert_eq!(
        status_cell(api(ProcStatus::Stopped, Some(true))),
        "disabled"
    );
}

/// fails if the word follows the mark alone: a sheep running while its
/// app is marked is a transient the shepherd should not paper over.
#[test]
fn a_stopped_sheep_that_is_not_disabled_still_reads_stopped() {
    assert_eq!(status_cell(api(ProcStatus::Stopped, None)), "stopped");
    assert_eq!(
        status_cell(api(ProcStatus::Stopped, Some(false))),
        "stopped"
    );
    assert_eq!(status_cell(api(ProcStatus::Online, Some(true))), "online");
}

#[test]
fn json_keeps_status_stopped_and_adds_the_mark() {
    let json = serde_json::to_value(FlockRows(vec![api(ProcStatus::Stopped, Some(true))])).unwrap();

    assert_eq!(json[0]["status"], "stopped");
    assert_eq!(json[0]["disabled"], true);
}

#[test]
fn json_has_no_disabled_key_on_a_sheep_that_was_never_disabled() {
    let json = serde_json::to_value(FlockRows(vec![api(ProcStatus::Stopped, None)])).unwrap();

    assert!(json[0].get("disabled").is_none(), "{json}");
}

#[test]
fn a_group_of_disabled_instances_reads_disabled() {
    let group = vec![
        ProcessInfo::builder(0, "api", ProcStatus::Stopped)
            .disabled(Some(true))
            .build(),
        ProcessInfo::builder(1, "api", ProcStatus::Stopped)
            .disabled(Some(true))
            .build(),
    ];

    assert_eq!(group_status(&group), "disabled");
}
