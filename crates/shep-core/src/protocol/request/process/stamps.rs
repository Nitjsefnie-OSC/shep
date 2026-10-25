//! The wire shape of a row's `log_timestamps` flag: absent from the payload
//! until a daemon has a stamps-off sheep to report.
//!
//! Sits beside [`super`] rather than inside it because `process.rs` is at
//! IR-48's file-size ceiling, and this is the field that has to grow there.

use super::*;

/// Every row that is not a stamps-off sheep is the bytes it always was.
#[test]
fn a_stamps_off_flag_is_absent_from_the_wire_until_set() {
    let plain = ProcessInfo::builder(1, "web", ProcStatus::Online).build();
    let json = serde_json::to_string(&plain).expect("serialize");
    assert!(
        !json.contains("log_timestamps"),
        "a row with nothing to say carries no key at all: {json}"
    );

    let unstamped = ProcessInfo::builder(1, "web", ProcStatus::Online)
        .log_timestamps(Some(false))
        .build();
    let json = serde_json::to_string(&unstamped).expect("serialize");
    assert!(json.contains(r#""log_timestamps":false"#), "{json}");
    let back: ProcessInfo = serde_json::from_str(&json).expect("round trip");
    assert_eq!(back.log_timestamps, Some(false));
}
