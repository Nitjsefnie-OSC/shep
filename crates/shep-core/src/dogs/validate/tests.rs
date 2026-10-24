use serde_json::{Map, json};

use super::*;

fn table(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> Map<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// The two ends of the grammar, in both directions: what shep sends is what
/// a dog reads back, for a sheep's table and for a dog's own section.
#[test]
fn a_proposed_table_survives_the_trip_through_params() {
    let sheep = Proposed::new(table([("keep", json!(2))]), Some("api".to_string()));
    assert_eq!(Proposed::parse(&sheep.params()), Some(sheep));

    let section = Proposed::new(table([("poll", json!("30s"))]), None);
    assert_eq!(Proposed::parse(&section.params()), Some(section));
}

#[test]
fn params_names_the_sheep_only_when_a_sheep_holds_the_table() {
    let params = Proposed::new(Map::new(), Some("api".to_string())).params();
    assert_eq!(
        serde_json::from_str::<Value>(&params).unwrap(),
        json!({"table": {}, "sheep": "api"})
    );

    let params = Proposed::new(Map::new(), None).params();
    assert_eq!(
        serde_json::from_str::<Value>(&params).unwrap(),
        json!({"table": {}}),
        "no sheep key at all, not a null one"
    );
}

/// A dog cannot check a table it cannot read, so every shape that is not what
/// shep sent answers `None` rather than an empty table.
#[test]
fn params_that_are_not_what_shep_sends_parse_as_nothing() {
    assert_eq!(Proposed::parse(""), None);
    assert_eq!(Proposed::parse("not json"), None);
    assert_eq!(Proposed::parse("[]"), None);
    assert_eq!(Proposed::parse(r#"{"table": 1}"#), None);
    assert_eq!(Proposed::parse(r#"{"sheep": "api"}"#), None);
    assert_eq!(
        Proposed::parse(r#"{"table": {}, "sheep": 7}"#),
        None,
        "a sheep that is not a name is not a proposed table"
    );
}

#[test]
fn a_verdict_survives_the_trip_through_the_reply_body() {
    for verdict in [
        Verdict::Accept,
        Verdict::refuse("this checkout belongs to another project"),
    ] {
        assert_eq!(Verdict::read(&verdict.body()), verdict);
    }
}

/// The safe direction: a dog that answers something that is not a verdict has
/// not accepted the write, and its own text is what the operator is told.
#[test]
fn an_answer_that_is_not_a_verdict_refuses() {
    for body in ["", "ok", "not json", "[]", r#"{"accept": "yes"}"#, "{}"] {
        let Verdict::Refuse { reason } = Verdict::read(body) else {
            panic!("{body:?} reads as an answer: {:?}", Verdict::read(body));
        };
        assert!(
            reason.starts_with("the dog answered with something"),
            "{reason}"
        );
    }
}

#[test]
fn a_refusal_without_a_reason_still_refuses() {
    assert_eq!(
        Verdict::read(r#"{"accept": false}"#),
        Verdict::refuse("the dog refused the write without saying why")
    );
    assert_eq!(
        Verdict::read(r#"{"accept": false, "reason": "  "}"#),
        Verdict::refuse("the dog refused the write without saying why"),
        "a blank reason says nothing, which is the same refusal"
    );
}

/// A quoted body is what the operator reads, so a dog that answers with a
/// whole file cannot make the refusal one.
#[test]
fn a_long_unreadable_answer_is_quoted_only_as_far_as_it_is_useful() {
    let body = "x".repeat(400);
    let Verdict::Refuse { reason } = Verdict::read(&body) else {
        panic!("an unreadable answer is a refusal");
    };
    assert!(reason.ends_with("..."), "{reason}");
    assert!(reason.len() < 260, "{reason}");
    assert!(
        !reason.contains(&"x".repeat(201)),
        "the whole body is quoted"
    );
}

/// A body cut mid-UTF-8 still quotes, and does not panic on the boundary.
#[test]
fn a_quote_cut_inside_a_multibyte_character_still_cuts() {
    let body = "é".repeat(200);
    let Verdict::Refuse { reason } = Verdict::read(&body) else {
        panic!("an unreadable answer is a refusal");
    };
    assert!(reason.ends_with("..."), "{reason}");
}

/// The table is a dog's settings, and a dog's settings hold credentials.
#[test]
fn proposed_debug_prints_a_key_count_and_no_value() {
    let proposed = Proposed::new(
        table([("webhook", json!("https://hunter2@hooks.example.com"))]),
        Some("api".to_string()),
    );
    let text = format!("{proposed:?}");

    assert_eq!(
        text, r#"Proposed { table: <1 key>, sheep: Some("api") }"#,
        "{text}"
    );
    assert!(!text.contains("hunter2"), "{text}");
}
