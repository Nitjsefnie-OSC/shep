//! The dog pane's write for a field inside an entry of an array or map of
//! tables: exact text, for each way `dogs.toml` can spell one.

use serde_json::{Value, json};

use super::super::edits::Edits;
use super::fixtures::{entries_schema, field};
use super::{ConfigPane, SectionWrite};

fn pane(section: &str) -> ConfigPane {
    ConfigPane::dog("paddock".into(), None, entries_schema(), section.to_owned())
}

fn edits_of(pairs: &[(&str, Value)]) -> Edits {
    let mut edits = Edits::default();
    for (key, value) in pairs {
        edits.set(field(key, value.clone()), None);
    }
    edits
}

fn written(pane: &ConfigPane, pairs: &[(&str, Value)]) -> String {
    match pane.edited_section_with(&edits_of(pairs)) {
        SectionWrite::Written(toml) => toml.as_str().to_owned(),
        other => panic!("wanted a section to send, got {other:?}"),
    }
}

/// Three `[[clients]]`, each with a comment above it, and one inside the
/// second.
const THREE: &str = concat!(
    "retries = 3\n",
    "\n",
    "# first\n",
    "[[clients]]\n",
    "name = \"alpha\"\n",
    "\n",
    "# second\n",
    "[[clients]]\n",
    "id = \"b-2\" # the id\n",
    "name = \"beta\"\n",
    "\n",
    "# third\n",
    "[[clients]]\n",
    "name = \"gamma\"\n",
);

#[test]
fn a_set_in_entry_one_of_three_keeps_the_comments_around_the_others() {
    let out = written(&pane(THREE), &[("clients[1].name", json!("bravo"))]);
    assert_eq!(out, THREE.replace("\"beta\"", "\"bravo\""));
}

#[test]
fn a_set_of_a_field_the_entry_lacks_joins_that_entry_alone() {
    let out = written(&pane(THREE), &[("clients[2].id", json!("c-3"))]);
    assert_eq!(out, format!("{THREE}id = \"c-3\"\n"));
    let out = written(&pane(THREE), &[("clients[0].id", json!("a-1"))]);
    assert_eq!(
        out,
        THREE.replace("name = \"alpha\"\n", "name = \"alpha\"\nid = \"a-1\"\n")
    );
}

#[test]
fn a_null_removes_that_key_of_that_entry_and_no_other() {
    let out = written(&pane(THREE), &[("clients[1].name", Value::Null)]);
    assert_eq!(out, THREE.replace("name = \"beta\"\n", ""));
}

#[test]
fn a_set_in_a_table_of_a_map_leaves_the_other_models() {
    let section = concat!(
        "# the small one\n",
        "[models.small]\n",
        "provider = \"local\"\n",
        "\n",
        "[models.gpt]\n",
        "provider = \"remote\"\n",
        "context = 4096\n",
    );
    let out = written(
        &pane(section),
        &[("models[\"gpt\"].provider", json!("edge"))],
    );
    assert_eq!(out, section.replace("\"remote\"", "\"edge\""));
}

#[test]
fn a_set_in_an_array_of_inline_tables_lands_in_that_inline_table() {
    let section = "clients = [{ name = \"alpha\" }, { name = \"beta\" }]\nretries = 3\n";
    let out = written(&pane(section), &[("clients[1].name", json!("bravo"))]);
    assert_eq!(
        out,
        "clients = [{ name = \"alpha\" }, { name = \"bravo\" }]\nretries = 3\n"
    );
}

#[test]
fn a_set_below_an_entry_creates_the_table_it_needs_inside_that_entry() {
    let out = written(&pane(THREE), &[("clients[1].limits.rate", json!(9))]);
    assert_eq!(
        out,
        THREE.replace(
            "name = \"beta\"\n",
            "name = \"beta\"\n\n[clients.limits]\nrate = 9\n"
        )
    );
}

#[test]
fn a_map_key_with_a_dot_is_one_key() {
    let section = "[models.\"a.b\"]\nprovider = \"local\"\n\n[models.a]\nprovider = \"other\"\n";
    let out = written(
        &pane(section),
        &[("models[\"a.b\"].provider", json!("edge"))],
    );
    assert_eq!(
        out,
        "[models.\"a.b\"]\nprovider = \"edge\"\n\n[models.a]\nprovider = \"other\"\n"
    );
}

#[test]
fn edits_to_two_entries_land_in_one_write() {
    let out = written(
        &pane(THREE),
        &[
            ("clients[0].name", json!("zero")),
            ("clients[2].name", json!("two")),
        ],
    );
    assert_eq!(
        out,
        THREE
            .replace("\"alpha\"", "\"zero\"")
            .replace("\"gamma\"", "\"two\"")
    );
}

/// The pane held three entries when the edit was filed; the section it
/// writes into holds two.
#[test]
fn an_index_past_the_end_of_the_array_writes_nothing() {
    let mut stale = pane(THREE);
    stale.section = Some(THREE.replace("# third\n[[clients]]\nname = \"gamma\"\n", ""));
    assert_eq!(
        stale.edited_section_with(&edits_of(&[("clients[2].name", json!("two"))])),
        SectionWrite::Refused
    );
    assert_eq!(
        stale.edited_section_with(&edits_of(&[("clients[2].name", Value::Null)])),
        SectionWrite::Refused
    );
}

/// A refresh rebuilds the rows, and a key whose entry is gone names no row.
/// It is not a top-level key spelled with brackets.
#[test]
fn a_filed_key_whose_entry_is_gone_writes_nothing() {
    let two = THREE.replace("# third\n[[clients]]\nname = \"gamma\"\n", "");
    assert_eq!(
        pane(&two).edited_section_with(&edits_of(&[("clients[2].name", json!("two"))])),
        SectionWrite::Refused
    );
}

#[test]
fn a_missing_map_entry_is_not_created() {
    let mut stale = pane("[models.small]\nprovider = \"local\"\n");
    stale.section = Some("[models.large]\nprovider = \"remote\"\n".to_owned());
    assert_eq!(
        stale.edited_section_with(&edits_of(&[("models[\"small\"].provider", json!("edge"))])),
        SectionWrite::Refused
    );
}
