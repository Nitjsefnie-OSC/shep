//! What `shep unset` plans: the keys it removes, the one request that removes
//! them, and what it refuses before anything is sent.

use serde_json::json;
use toml_edit::DocumentMut;

use super::*;
use crate::commands::copy::plan::ChangeKind;

/// A sheep's config with one dog table, spelled as a Flockfile spells it.
fn config() -> Json {
    json!({
        "name": "api",
        "max_memory": "512M",
        "dogs": {
            "kelpie": { "git": { "checkout": "/srv/api", "merging": "ff" } }
        }
    })
}

fn keys(path: &str) -> Vec<String> {
    path.split('.').map(str::to_string).collect()
}

/// fails if dropping an override stops being one `ResetSheepField` that lists
/// the value the override holds as removed. The value that returns is the
/// shepherd's to say, so the list carries no new value.
#[test]
fn an_overridden_field_plans_one_reset_that_lists_the_old_value() {
    let plan = plan_field("api", "max_memory", &config(), &["max_memory".to_string()]).unwrap();

    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].key, "max_memory");
    assert_eq!(plan.changes[0].kind, ChangeKind::Removed);
    assert_eq!(plan.changes[0].old, Some(json!("512M")));
    assert_eq!(plan.changes[0].new, None);
    let Write::ResetSheepField { key } = &plan.write else {
        panic!("a field's override is dropped through ResetSheepField");
    };
    assert_eq!(key, "max_memory");
}

/// fails if a field the operator never overrode reads as a removal. Nothing
/// would be dropped, and a mistyped key should be told so.
#[test]
fn a_field_with_no_override_is_refused_with_the_reader_that_shows_it() {
    let error = plan_field(
        "api",
        "max_memory",
        &config(),
        &["restart_delay".to_string()],
    )
    .unwrap_err();

    let WriteRefusal::Refused(text) = &error else {
        panic!("expected a refusal, got {error:?}");
    };
    assert!(
        text.contains("`api.max_memory` has no override to drop"),
        "{text}"
    );
    assert!(text.contains("`shep get api.max_memory`"), "{text}");
    assert_eq!(error.code(), ExitCode::InvalidConfig);
}

/// fails if a field that holds `null` under an override can no longer be
/// dropped. The change list would be empty and the run would announce that
/// there is nothing to remove.
#[test]
fn an_override_holding_null_still_lists_its_removal() {
    let config = json!({ "cwd": null });
    let plan = plan_field("api", "cwd", &config, &["cwd".to_string()]).unwrap();

    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].kind, ChangeKind::Removed);
}

/// fails if removing one key of a sheep's dog table sends anything but the
/// table without it, or lists more than the one key.
#[test]
fn a_dog_table_key_leaves_the_table_and_its_neighbours_stay() {
    let plan = plan_dog_table("api", "kelpie", &keys("git.merging"), &config()).unwrap();

    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].key, "merging");
    assert_eq!(plan.changes[0].kind, ChangeKind::Removed);
    assert_eq!(plan.changes[0].old, Some(json!("ff")));
    let Write::SheepDogTable { dog, table, before } = &plan.write else {
        panic!("a key of the table writes the table without it");
    };
    assert_eq!(dog, "kelpie");
    assert_eq!(table.as_map()["git"], json!({ "checkout": "/srv/api" }));
    assert_eq!(before["git"]["merging"], json!("ff"));
}

/// fails if naming the whole dog table stops removing the table, or lists
/// anything but the keys in it.
#[test]
fn a_whole_dog_table_is_removed_with_no_table_sent() {
    let plan = plan_dog_table("api", "kelpie", &[], &config()).unwrap();

    assert!(matches!(&plan.write, Write::DropSheepDogTable { dog, .. } if dog == "kelpie"));
    let listed: Vec<&str> = plan.changes.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(listed, ["git.checkout", "git.merging"]);
    assert!(plan.changes.iter().all(|c| c.kind == ChangeKind::Removed));
}

/// fails if a key the table does not hold, or a dog the sheep holds no table
/// for, is planned as a removal that would report success.
#[test]
fn a_key_or_table_nothing_holds_is_refused() {
    let key = plan_dog_table("api", "kelpie", &keys("git.ghost"), &config()).unwrap_err();
    assert!(
        key.to_string()
            .contains("`api.dogs.kelpie.git.ghost` holds nothing to remove"),
        "{key}"
    );

    let through =
        plan_dog_table("api", "kelpie", &keys("git.checkout.deeper"), &config()).unwrap_err();
    assert!(matches!(through, WriteRefusal::Refused(_)), "{through:?}");

    let table = plan_dog_table("api", "deploy", &[], &config()).unwrap_err();
    assert!(
        table
            .to_string()
            .contains("`api.dogs.deploy` holds nothing to remove"),
        "{table}"
    );
}

fn section() -> DocumentMut {
    "# above the header\n[bark]\npoll = \"30s\"\n# the other key\nhistory_bytes = 1048576\n\n\
     [bark.sinks]\nurl = \"x\"\n"
        .parse()
        .unwrap()
}

/// fails if removing a dog's key renders the section from a map instead of
/// editing the text read. The comments around the neighbours are the operator's.
#[test]
fn a_section_key_is_removed_from_the_text_in_place() {
    let plan = plan_section(section(), "bark", &keys("bark.poll")).unwrap();

    let Write::DogSection { toml, before } = &plan.write else {
        panic!("a dog's key is written as its section's text");
    };
    assert!(!toml.contains("poll"), "{toml}");
    assert!(toml.contains("# above the header"), "{toml}");
    assert!(
        toml.contains("# the other key\nhistory_bytes = 1048576"),
        "{toml}"
    );
    assert!(
        before.contains("poll = \"30s\""),
        "the text read is kept for the staleness check"
    );
    assert_eq!(plan.changes.len(), 1);
    assert_eq!(plan.changes[0].key, "poll");
    assert_eq!(plan.changes[0].old, Some(json!("30s")));
}

/// fails if a nested key is not found under the table that holds it, or if
/// removing a table drops anything but the leaves under it.
#[test]
fn a_nested_section_key_and_a_whole_table_both_leave() {
    let nested = plan_section(section(), "bark", &keys("bark.sinks.url")).unwrap();
    let Write::DogSection { toml, .. } = &nested.write else {
        panic!("a section write");
    };
    assert!(!toml.contains("url"), "{toml}");

    let table = plan_section(section(), "bark", &keys("bark.sinks")).unwrap();
    let Write::DogSection { toml, .. } = &table.write else {
        panic!("a section write");
    };
    assert!(!toml.contains("sinks"), "{toml}");
    assert_eq!(table.changes[0].key, "url");
}

/// fails if naming a whole section removes it, or a key it does not hold reads
/// as a removal.
#[test]
fn a_whole_section_and_a_missing_key_are_refused() {
    let whole = plan_section(section(), "bark", &[]).unwrap_err();
    assert!(matches!(whole, WriteRefusal::Usage(_)), "{whole:?}");
    assert_eq!(whole.code(), ExitCode::Usage);

    let missing = plan_section(section(), "bark", &keys("ghost")).unwrap_err();
    assert!(
        missing
            .to_string()
            .contains("`bark.ghost` holds nothing to remove"),
        "{missing}"
    );
}

/// fails if `unset` words its question or its stale-holder refusal as `set`.
#[test]
fn the_question_and_the_stale_refusal_name_the_verb() {
    use std::io::Cursor;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut streams = Streams {
        out: &mut out,
        err: &mut err,
        style: crate::style::Presentation::BARE,
        fmt: crate::cli::Format::Table,
    };
    let mut reader = Cursor::new(b"n\n".as_slice());
    super::super::flow::ask(
        &mut streams,
        Verb::Unset,
        "api",
        "max_memory",
        1,
        &mut reader,
    );
    let question = String::from_utf8_lossy(&err).to_string();
    assert!(
        question.contains("unset `api.max_memory`? [y/N]"),
        "{question}"
    );

    let write = Write::DropSheepDogTable {
        dog: "kelpie".to_string(),
        before: Json::Null,
    };
    let text = super::super::flow::holder_moved(Verb::Unset, "api", "dogs.kelpie", &write);
    assert!(text.contains("`shep unset` waited"), "{text}");
    assert!(text.contains("`api`'s table for `kelpie`"), "{text}");
}
