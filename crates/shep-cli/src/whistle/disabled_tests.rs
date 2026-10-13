//! `SheepRow` carries a disabled sheep's mark exactly as `ProcessInfo` does.

use shep_core::protocol::ProcessInfo;
use shep_core::status::ProcStatus;

use super::facts::SheepRow;

fn parked(disabled: Option<bool>) -> ProcessInfo {
    ProcessInfo::builder(4, "api", ProcStatus::Stopped)
        .disabled(disabled)
        .build()
}

#[test]
fn a_disabled_sheep_row_serializes_exactly_as_process_info_does() {
    let info = parked(Some(true));
    let row = serde_json::to_value(SheepRow::from(&info)).unwrap();

    assert_eq!(row, serde_json::to_value(&info).unwrap());
    assert_eq!(row["status"], "stopped");
    assert_eq!(row["disabled"], true);
}

/// fails if the twin writes `disabled: null`: `ProcessInfo` leaves the key
/// out of every row that is not disabled.
#[test]
fn a_sheep_row_that_is_not_disabled_has_no_disabled_key() {
    let info = parked(None);
    let row = serde_json::to_value(SheepRow::from(&info)).unwrap();

    assert_eq!(row, serde_json::to_value(&info).unwrap());
    assert!(row.get("disabled").is_none(), "{row}");
}

#[test]
fn the_generated_schema_names_disabled() {
    let schema = serde_json::to_value(schemars::schema_for!(SheepRow)).unwrap();

    assert!(schema["properties"].get("disabled").is_some(), "{schema}");
}
