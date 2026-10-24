//! [`super::table_refusal`]: what a proposed dog table has to be, and what
//! it is refused for.
//!
//! Every case is driven through a schema spelled the way a dog's `--schema`
//! answer is spelled, through a `$ref` and an `anyOf`, so a test proves the
//! gate reads the schema and not a fixture shaped like one.

use serde_json::{Map, Value, json};

use super::table_refusal;

/// A dog's own section schema: a bounded integer, a list whose item schema
/// publishes `minLength`, a closed set, a nested table with a `pattern`, an
/// array of tables, a tagged union, a secret with a grammar of the dog's own,
/// and a duration spelled `anyOf: [{$ref}, {type: null}]` the way
/// `shep-log-rotate` spells one.
fn schema() -> Value {
    json!({
        "$ref": "#/$defs/Settings",
        "$defs": {
            "Settings": {
                "type": "object",
                "properties": {
                    "keep": { "type": "integer", "minimum": 1, "maximum": 64 },
                    "tags": { "type": "array", "items": { "type": "string", "minLength": 3 } },
                    "merge": { "enum": ["ask", "auto"] },
                    "hours": {
                        "type": "object",
                        "properties": {
                            "start": { "type": "string", "pattern": "^\\d{2}:\\d{2}$" },
                        },
                    },
                    "hosts": { "type": "array", "items": { "$ref": "#/$defs/Host" } },
                    "retro": { "$ref": "#/$defs/Step" },
                    "token": { "type": "string", "x-shep-secret": true, "pattern": "^sk-" },
                    "region": { "type": "string", "pattern": "^eu-" },
                    "max_age": {
                        "anyOf": [{ "$ref": "#/$defs/UpDuration" }, { "type": "null" }],
                    },
                },
            },
            "Host": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "weight": { "type": "integer", "minimum": 1 },
                },
            },
            "Step": {
                "oneOf": [
                    {
                        "type": "object",
                        "properties": { "kind": { "type": "string", "const": "none" } },
                        "required": ["kind"],
                    },
                    {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "const": "path" },
                            "path": { "type": "string", "pattern": "^skills/" },
                        },
                        "required": ["kind", "path"],
                    },
                    {
                        "type": "object",
                        "properties": {
                            "kind": { "type": "string", "const": "plugin" },
                            "plugin": { "type": "string" },
                            "depth": { "type": "integer", "minimum": 1, "maximum": 3 },
                        },
                        "required": ["kind", "plugin"],
                    },
                ],
            },
            "UpDuration": { "type": "string", "pattern": "^\\d+(ms|h|m|s)?$" },
        },
    })
}

/// `table` as the map [`table_refusal`] takes.
fn table(value: Value) -> Map<String, Value> {
    value.as_object().expect("a table is an object").clone()
}

/// The whole table the schema takes, which is the case a gate that refuses
/// everything would also pass.
fn accepted() -> Value {
    json!({
        "keep": 8,
        "tags": ["ask", "auto"],
        "merge": "ask",
        "hours": { "start": "09:30" },
        "hosts": [{ "name": "build", "weight": 2 }],
        "retro": { "kind": "plugin", "plugin": "retro", "depth": 2 },
        "token": "sk-live-51Hx9Qa",
        "max_age": "2h",
    })
}

#[test]
fn a_table_the_schema_takes_is_refused_nowhere() {
    assert_eq!(table_refusal(&schema(), &table(accepted())), None);
}

#[test]
fn a_scalar_out_of_its_floor_is_refused_with_its_path_and_sentence() {
    let refused = table_refusal(&schema(), &table(json!({ "keep": 0 }))).expect("0 is under 1");
    assert_eq!(refused.path, ["keep"]);
    assert_eq!(refused.text, "keep is \"0\"; keep starts at 1");
}

/// The same arm [`crate::dog_settings::refusal`] runs for a typed buffer,
/// answering a value that is already a number: the refusal is the sentence
/// the pane would have printed, not a second opinion about the type.
#[test]
fn a_scalar_that_is_not_a_number_is_refused_in_the_panes_own_words() {
    let refused = table_refusal(&schema(), &table(json!({ "keep": "lots" })))
        .expect("\"lots\" is not a whole number");
    assert_eq!(refused.path, ["keep"]);
    assert_eq!(
        refused.text,
        "keep is \"lots\", which is not a whole number shep accepts"
    );
}

#[test]
fn a_closed_set_refuses_a_value_it_does_not_name() {
    let refused = table_refusal(&schema(), &table(json!({ "merge": "never" })))
        .expect("never is not in the set");
    assert_eq!(refused.path, ["merge"]);
    assert_eq!(
        refused.text,
        "merge is \"never\"; merge takes one of ask, auto"
    );
}

/// The element, not the array, is what the item schema bounds, so the path
/// names the element and the sentence quotes it.
#[test]
fn a_list_element_the_item_schema_refuses_is_named_with_its_index() {
    let refused = table_refusal(&schema(), &table(json!({ "tags": ["ask", "ab"] })))
        .expect("ab is under the item schema's minLength");
    assert_eq!(refused.path, ["tags", "1"]);
    assert_eq!(
        refused.text,
        "tags.1 is \"ab\"; tags.1 needs at least 3 characters"
    );
}

#[test]
fn a_value_where_a_list_belongs_is_refused_as_a_list() {
    let refused =
        table_refusal(&schema(), &table(json!({ "tags": "ask" }))).expect("a string is not a list");
    assert_eq!(refused.path, ["tags"]);
    assert_eq!(
        refused.text,
        "tags is \"ask\", which is not a list shep accepts"
    );
}

/// A union is one table, so the refusal for a tag no arm holds a `const` on
/// names the union and lists the tags it does take.
#[test]
fn a_union_that_names_no_kind_is_refused_with_the_kinds_it_takes() {
    let refused = table_refusal(&schema(), &table(json!({ "retro": { "kind": "robot" } })))
        .expect("no arm holds a const on robot");
    assert_eq!(refused.path, ["retro"]);
    assert_eq!(
        refused.text,
        "retro is \"robot\", which is not a kind retro takes; try one of none, path, plugin"
    );
}

#[test]
fn a_union_that_names_no_tag_at_all_says_so() {
    let refused = table_refusal(&schema(), &table(json!({ "retro": { "plugin": "retro" } })))
        .expect("the arm's fields are not the tag");
    assert_eq!(refused.path, ["retro"]);
    assert_eq!(
        refused.text,
        "retro names no kind of its own; try one of none, path, plugin"
    );
}

/// The arm the tag names is the one whose fields are checked: an out-of-range
/// `depth` belongs to `plugin`, and a `path` held by the `path` arm is not
/// asked about here.
#[test]
fn a_field_of_the_chosen_arm_is_held_to_its_own_bounds() {
    let refused = table_refusal(
        &schema(),
        &table(json!({ "retro": { "kind": "plugin", "plugin": "retro", "depth": 9 } })),
    )
    .expect("9 is over the arm's ceiling");
    assert_eq!(refused.path, ["retro", "depth"]);
    assert_eq!(refused.text, "retro.depth is \"9\"; retro.depth stops at 3");

    let refused = table_refusal(
        &schema(),
        &table(json!({ "retro": { "kind": "path", "path": "mine" } })),
    )
    .expect("mine does not match the arm's pattern");
    assert_eq!(refused.path, ["retro", "path"]);
    assert_eq!(
        refused.text,
        "retro.path is \"mine\", which it does not take; try it matches ^skills/"
    );
}

/// A nested table is not one leaf: the row is `hours.start`, and the path
/// names every table the value sits in.
#[test]
fn a_nested_tables_leaf_is_refused_at_its_whole_path() {
    let refused = table_refusal(&schema(), &table(json!({ "hours": { "start": "9am" } })))
        .expect("9am does not match the pattern");
    assert_eq!(refused.path, ["hours", "start"]);
    assert_eq!(
        refused.text,
        "hours.start is \"9am\", which it does not take; try it matches ^\\d{2}:\\d{2}$"
    );
}

/// The other half of a nested table: a scalar sitting where the schema has a
/// table leaves every leaf below it unreadable, and the key to fix is the one
/// the operator wrote.
#[test]
fn a_scalar_where_the_schema_has_a_table_is_refused_at_the_table() {
    let refused = table_refusal(&schema(), &table(json!({ "hours": "09:30" })))
        .expect("a string is not a table");
    assert_eq!(refused.path, ["hours"]);
    assert_eq!(
        refused.text,
        "hours is \"09:30\", which is not a table shep accepts"
    );
}

/// An array of tables is one row and many entries: the item schema flattened
/// beside it is what bounds each entry's own fields.
#[test]
fn an_entry_of_an_array_of_tables_is_refused_at_its_index() {
    let refused = table_refusal(
        &schema(),
        &table(json!({ "hosts": [{ "name": "build" }, { "name": "web", "weight": 0 }] })),
    )
    .expect("weight 0 is under the item schema's floor");
    assert_eq!(refused.path, ["hosts", "1", "weight"]);
    assert_eq!(
        refused.text,
        "hosts.1.weight is \"0\"; hosts.1.weight starts at 1"
    );
}

/// shep's own grammar, reached through a `$ref` behind an `anyOf`, which is
/// how a dog spells `Option<UpDuration>`. `1d` is the natural spelling of a
/// retention window and shep's grammar tops out at hours.
#[test]
fn a_nullable_grammar_refuses_a_spelling_shep_does_not_take() {
    let refused = table_refusal(&schema(), &table(json!({ "max_age": "1d" })))
        .expect("1d is outside the grammar");
    assert_eq!(refused.path, ["max_age"]);
    assert_eq!(
        refused.text,
        "max_age is \"1d\", which is not a duration shep accepts; \
         try 500ms, 2s, 5m, 1h, a bare number is milliseconds"
    );
}

/// A secret's value must not come back at the caller: the schema decides that
/// a field is a credential, and every sentence this gate builds resolves what
/// it may print before it builds one.
#[test]
fn a_secrets_value_never_reaches_the_sentence() {
    const CREDENTIAL: &str = "sk_live_HUNTER2";
    let refused = table_refusal(&schema(), &table(json!({ "token": CREDENTIAL })))
        .expect("the pattern refuses it");
    assert_eq!(
        refused.text,
        "token is \"<set>\", which it does not take; try it matches ^sk-"
    );
    assert!(!refused.text.contains(CREDENTIAL), "{}", refused.text);
    assert!(!format!("{refused:?}").contains(CREDENTIAL), "{refused:?}");
}

/// The control on the test above: a value that is not a credential is quoted
/// back, since redacting everything would pass that test while making every
/// refusal useless.
#[test]
fn an_ordinary_value_is_still_quoted_back() {
    let refused = table_refusal(&schema(), &table(json!({ "region": "Europe" })))
        .expect("Europe does not match ^eu-");
    assert_eq!(refused.path, ["region"]);
    assert!(refused.text.contains("Europe"), "{}", refused.text);
}

/// A write carries the whole table, keys the schema has never heard of
/// included, and shep is not the authority on a dog's config beyond what that
/// dog published.
#[test]
fn a_key_the_schema_does_not_name_is_left_alone() {
    assert_eq!(
        table_refusal(
            &schema(),
            &table(
                json!({ "keep": 8, "operator_wrote_this": "anything", "hours": { "start": "09:30", "extra": 7 } })
            )
        ),
        None
    );
}

/// An absent key and a `null` are both unset, which for a dog is the way a
/// value goes back to the dog's own default.
#[test]
fn a_null_is_unset_and_not_a_value_to_check() {
    assert_eq!(
        table_refusal(
            &schema(),
            &table(
                json!({ "keep": null, "tags": null, "retro": null, "max_age": null, "hours": null })
            )
        ),
        None
    );
}

/// A read-only row is a row this pane has no editor for: a list whose items
/// may be `null`, and a map of arbitrary keys. Neither is checked, exactly as
/// neither is edited.
#[test]
fn a_read_only_row_refuses_nothing() {
    let schema = json!({
        "type": "object",
        "properties": {
            "maybe": { "type": "array", "items": { "type": ["string", "null"] } },
            "labels": { "type": "object", "additionalProperties": { "type": "string" } },
        },
    });
    assert_eq!(
        table_refusal(
            &schema,
            &table(json!({ "maybe": [1, 2], "labels": { "any": 7 } }))
        ),
        None
    );
}

/// The first refusal, in row order, and nothing after it.
#[test]
fn the_first_refusal_in_row_order_is_the_only_answer() {
    let refused = table_refusal(&schema(), &table(json!({ "keep": 0, "merge": "never" })))
        .expect("two refusals are available");
    assert_eq!(refused.path, ["keep"]);
}

/// IR-41: the `Debug` decision is derived, and the exact string is what pins
/// it.
#[test]
fn the_debug_of_a_refusal_names_the_path_and_the_sentence() {
    let refused = table_refusal(&schema(), &table(json!({ "keep": 0 }))).expect("0 is under 1");
    assert_eq!(
        format!("{refused:?}"),
        "TableRefusal { path: [\"keep\"], text: \"keep is \\\"0\\\"; keep starts at 1\" }"
    );
}
