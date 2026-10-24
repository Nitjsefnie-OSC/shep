//! The value half of `shep set`: what one string from the command line parses
//! as, and where the TOML grammar ends.
//!
//! The plan half is in `tests.rs`, and the live cases are in
//! `tests/cli_e2e/settings_set.rs`.

use serde_json::json;

use super::*;

/// `raw` as the value `shep set` takes it, for the tests that are not about a
/// value it refuses.
#[track_caller]
fn ok_value(raw: &str) -> Json {
    value_of(raw).unwrap()
}

/// The value `shep set <address> <raw>` hands the shepherd.
#[test]
fn a_value_is_toml_and_other_text_is_the_string_as_typed() {
    assert_eq!(ok_value("5"), json!(5));
    assert_eq!(ok_value("-2"), json!(-2));
    assert_eq!(ok_value("1.5"), json!(1.5));
    assert_eq!(ok_value("true"), json!(true));
    assert_eq!(ok_value("false"), json!(false));
    assert_eq!(ok_value("[\"a\", \"b\"]"), json!(["a", "b"]));
    assert_eq!(ok_value("{ keep = 1 }"), json!({ "keep": 1 }));
    assert_eq!(ok_value("\"5\""), json!("5"));
    // A duration, a bare word and a line of TOML that is not a value are all
    // strings: `3s` is what an operator typed, and `512M` is what a Flockfile
    // says for the same setting.
    assert_eq!(ok_value("3s"), json!("3s"));
    assert_eq!(ok_value("auto"), json!("auto"));
    assert_eq!(ok_value("keep = 1"), json!("keep = 1"));
    assert_eq!(ok_value(""), json!(""));
    // A table nested in a table, because a whole dog table is one value.
    assert_eq!(
        ok_value("{ git = { merging = \"ff\" } }"),
        json!({ "git": { "merging": "ff" } })
    );
}
/// A TOML date, time or datetime reads as the text it spells, which is what
/// shep's JSON plane carries one as everywhere else, and never as a null.
#[test]
fn a_toml_datetime_is_the_text_it_spells() {
    assert_eq!(
        ok_value("1979-05-27T07:32:00Z"),
        json!("1979-05-27T07:32:00Z")
    );
    assert_eq!(ok_value("1979-05-27"), json!("1979-05-27"));
    assert_eq!(ok_value("07:32:00"), json!("07:32:00"));

    let section = "since = 1979-05-27T07:32:00Z\nkeep = 1\n";
    let plan = plan_section(
        section.parse::<toml_edit::DocumentMut>().unwrap(),
        "kelpie",
        &["keep".to_string()],
        &json!(2),
    )
    .unwrap();
    assert_eq!(plan.table["since"], json!("1979-05-27T07:32:00Z"));

    // A whole-section write keeps the datetime the section already held, which
    // a null would have dropped on the way back out.
    let plan = plan_section(
        section.parse::<toml_edit::DocumentMut>().unwrap(),
        "kelpie",
        &[],
        &json!({ "since": "1979-05-27T07:32:00Z", "keep": 1 }),
    )
    .unwrap();
    let Write::DogSection { toml, .. } = &plan.write else {
        unreachable!()
    };
    assert!(toml.contains("since = \"1979-05-27T07:32:00Z\""), "{toml}");
    assert!(!toml.contains("null"), "{toml}");
}
/// TOML spells `nan`, `inf` and `-inf`, and the JSON the shepherd's requests
/// carry have no cell for them: `serde_json` renders a non-finite number as a
/// null. A value that parses as TOML's `nan` would reach the flock as a write of
/// nothing.
#[test]
fn a_non_finite_number_is_refused_as_a_value() {
    for raw in ["nan", "inf", "+inf", "-inf"] {
        let error = value_of(raw).expect_err("`{raw}` has no JSON cell");
        assert_eq!(error.code(), ExitCode::Usage);
        assert!(
            error.to_string().contains("no place in the JSON"),
            "{error}"
        );
        assert!(error.to_string().contains("null"), "{error}");
    }

    // The quoted text is a string, which is the remedy the sentence gives, and a
    // finite float is still a float.
    assert_eq!(ok_value("\"nan\""), json!("nan"));
    assert_eq!(ok_value("1.5"), json!(1.5));
    assert_eq!(ok_value("6.02e23"), json!(6.02e23));
    assert_eq!(ok_value("nan-ish"), json!("nan-ish"), "text, not a float");
}

/// A `nan` inside a table or an array value is refused too, and named where it
/// sits, because the operator has to find it inside what they typed.
#[test]
fn a_non_finite_number_inside_a_value_is_named_where_it_sits() {
    let error = value_of("{ keep = nan }").expect_err("a nan inside a table is a nan");
    assert!(error.to_string().contains("under `keep`"), "{error}");

    let error = value_of("{ hours = { start = inf } }").expect_err("two levels down");
    assert!(error.to_string().contains("under `hours.start`"), "{error}");

    let error = value_of("[1, -inf]").expect_err("inside an array");
    assert!(error.to_string().contains("under `[1]`"), "{error}");
}

/// A `nan` a section already holds is what the change list has to report, and it
/// is not a null: a key-level write leaves the cell where it was.
#[test]
fn a_nan_a_section_already_holds_reads_as_the_text_it_spells() {
    let section = "keep = nan\nmerge = \"ask\"\n";
    let plan = plan_section(
        section.parse::<toml_edit::DocumentMut>().unwrap(),
        "kelpie",
        &["merge".to_string()],
        &json!("auto"),
    )
    .unwrap();
    assert_eq!(plan.table["keep"], json!("nan"));
    assert_eq!(plan.changes.len(), 1, "only the key named changed");
    let Write::DogSection { toml, .. } = &plan.write else {
        unreachable!()
    };
    assert!(
        toml.contains("keep = nan"),
        "{toml}: the cell is left alone"
    );
}
