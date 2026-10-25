//! The wire shape of `ResetSheepField` and its reply.

use serde_json::json;

use super::{Request, Response};

/// fails if the request's tag or payload names move. A reader written against
/// this shape sends `reset_sheep_field` with a sheep and a key, and nothing
/// else: the value that comes back is the shepherd's to know.
#[test]
fn reset_sheep_field_names_a_sheep_and_a_key() {
    let request = Request::ResetSheepField {
        name: "web".to_string(),
        key: "max_restarts".to_string(),
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({ "kind": "reset_sheep_field", "name": "web", "key": "max_restarts" })
    );
}

/// fails if the reply stops carrying the restored value, or if a `None`
/// warning appears on the wire: the field is additive, as `SheepFieldSet`'s is.
#[test]
fn the_reply_carries_the_value_the_field_went_back_to() {
    let reply = Response::SheepFieldReset {
        name: "web".to_string(),
        key: "max_restarts".to_string(),
        value: json!(16),
        pending: false,
        warning: None,
    };
    let wire = serde_json::to_value(&reply).unwrap();
    assert_eq!(wire["data"]["value"], json!(16));
    assert!(wire["data"].get("warning").is_none(), "{wire}");
    assert_eq!(
        serde_json::from_value::<Response>(wire).unwrap(),
        reply,
        "the reply must read back as it was sent"
    );
    assert_eq!(reply.name(), "SheepFieldReset");
}
