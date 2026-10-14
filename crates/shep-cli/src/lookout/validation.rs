//! What a field accepts, in the operator's words.
//!
//! Two sources, and the per-field one wins outright rather than merging:
//! a field writing its own `accepts` gets those and none from the table.
//! One rule beats merge semantics nobody can predict. The table itself is
//! [`forms`], and the per-field lists are backed one crate down, where
//! shep-core hands `normalize` a value for every refusal it makes, and
//! names the enforcer for the few decided at spawn.
//!
//! [`refusal`] is the other half, and it is here rather than in the pane so
//! that the sentence an operator reads and the check that stops the write
//! are built from one table. A dog's section is written by the shepherd
//! without being read, so there is no second gate behind this one.
//! [`item_refusal`] is the same gate for one element of a list field,
//! against the bounds its item schema published.
//!
//! The arms of that gate live one level down, each with the tests for what it
//! checks: [`numbers`] for the whole-number and decimal parse and bounds,
//! [`items`] for one element of a list field, [`patterns`] for a dog's own
//! grammar. What is here is the door itself, the sentence, and the rule about
//! what a sentence may print.

#[cfg(test)]
mod fixtures;
mod forms;
mod items;
mod numbers;
mod patterns;

pub use forms::bullets;
pub use items::item_refusal;

use shep_core::values::{MemSize, UpDuration};

use serde_json::Value;

use super::field::{Bounds, Field, FieldKind, ValueKind};
use forms::{DURATION_FORMS, Form, MEMORY_FORMS};
use numbers::{integer_refusal, number_refusal};
use patterns::pattern_refusal;

/// What a refusal prints in place of a secret's value.
///
/// The same marker the pane already draws for a secret it holds, so an
/// operator reads one word for one idea rather than learning a second.
pub const REDACTED: &str = "<set>";

/// What may be printed for `typed`: [`REDACTED`] when `secret`, the value
/// itself otherwise.
///
/// Every sentence this module builds resolves its value here, so a refusal
/// added later cannot echo a secret by forgetting to ask. The rule is one
/// line and the reason it is a function is that it has three callers.
///
/// `pub(super)`: `ConfigPane::apply_typing` builds one refusal of its own
/// through [`not_a_whole_number`], and has to resolve the value the same
/// way.
#[must_use]
pub(super) fn shown_value(secret: bool, typed: &str) -> &str {
    if secret { REDACTED } else { typed }
}

/// Why a typed buffer was not filed, in the operator's words.
///
/// One sentence, ready for the status bar: it names the key, quotes what
/// was typed, and says what the field does take. The house shape every
/// other refusal in the pane follows.
///
/// `Debug` is derived (IR-41), which is safe only because [`refusal`]
/// redacts before it builds one: a secret's value never reaches `text`,
/// so there is nothing here for a hand-written impl to hide. Both halves
/// are pinned by exact-string tests below.
///
/// The value quoted for a field that is not secret is one the operator
/// just typed, and the row beside it is already drawing the same thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The sentence to print.
    pub text: String,
}

/// Whether `field` takes `typed`, and why not when it does not.
///
/// Five checks, and a field reaches at most one of them:
///
/// - A [`FieldKind::Integer`] buffer must parse as an `i64` and sit inside
///   whatever `minimum`, `maximum`, `exclusiveMinimum` and `exclusiveMaximum`
///   the schema published, the two strict spellings folded into the pair at the
///   next integer in.
/// - A [`FieldKind::Number`] buffer must parse as a finite `f64` and sit inside
///   whatever `minimum`, `maximum`, `exclusiveMinimum` and `exclusiveMaximum`
///   the schema published, each read as the schema wrote it.
/// - A string field naming one of shep-core's own grammars goes through
///   that type's `FromStr`, which is the enforcer itself rather than a
///   second opinion about it.
/// - Any other string field with a `pattern` is matched against it. This
///   is the only door a third-party dog's own grammar has, since shep has
///   no parser for a type it has never heard of.
/// - A numeric field whose `enum` this pane could not turn into a cycle
///   widget, which is what a set of numbers is, is held to that set by
///   [`enum_refusal`] once its value parsed. A set of strings is
///   [`FieldKind::Choice`] instead, chosen from rather than typed, so it
///   never reaches this door at all.
///
/// [`None`] for everything else, which is most fields: a plain string with
/// no pattern takes any string, and shep is not the authority on a dog's
/// config beyond what that dog published. A list field's element is checked
/// by [`item_refusal`] rather than here, against its item schema.
///
/// Fails open on a `pattern` that does not compile. A dog that ships a
/// broken regex has a bug in its schema, and refusing every value for that
/// field would lock an operator out of a setting on the strength of
/// somebody else's typo.
///
/// The empty buffer never reaches here: it means "unset", which is
/// `ConfigPane::apply_typing`'s own case and not a value to check.
#[must_use]
pub fn refusal(field: &Field, typed: &str) -> Option<Refusal> {
    if typed.is_empty() {
        return None;
    }
    // Every path below quotes `typed` back at the operator, and
    // `App::on_pane_text_key` paints the sentence in the status bar. For a
    // secret that is the one place in this pane the value would appear:
    // `ConfigPane::begin_typing` seeds a secret's editor empty and the row
    // renders `<set>`, both so the value is never drawn. A schema decides
    // this, not shep, so it is reachable rather than theoretical: a dog
    // publishing `pattern` or a `$ref` grammar on a field it also marks
    // `x-shep-secret` sends a credential straight through `pattern_refusal`.
    let shown = shown_value(field.secret, typed);
    if field.kind == FieldKind::Integer {
        return integer_refusal(&field.key, &field.bounds, typed, shown);
    }
    if field.kind == FieldKind::Number {
        return number_refusal(&field.key, &field.bounds, typed, shown);
    }
    match field.value_kind {
        Some(ValueKind::UpDuration) => typed
            .parse::<UpDuration>()
            .is_err()
            .then(|| refuse(&field.key, shown, "a duration", DURATION_FORMS)),
        Some(ValueKind::MemSize) => typed
            .parse::<MemSize>()
            .is_err()
            .then(|| refuse(&field.key, shown, "a size", MEMORY_FORMS)),
        None => pattern_refusal(
            &field.key,
            field.bounds.pattern.as_deref(),
            &field.accepts,
            typed,
            shown,
        ),
    }
}

/// The refusal for a [`FieldKind::Integer`] buffer that is not a number.
///
/// Not built from [`forms::INTEGER_FORMS`] through [`refuse`], which would append
/// "try a whole number" to a sentence that has just said "is not a whole
/// number". The table's own text still draws the panel, where "a whole
/// number" is the whole answer and reads correctly.
///
/// `pub(super)`: `ConfigPane::apply_typing`'s own integer arm answers with
/// this, so the one sentence has one spelling.
///
/// `shown` is what may be printed: [`REDACTED`] rather than the value when
/// the field is a credential. Callers pass what [`refusal`] resolved.
pub(super) fn not_a_whole_number(key: &str, shown: &str) -> Refusal {
    Refusal {
        text: format!("{key} is \"{shown}\", which is not a whole number shep accepts"),
    }
}

/// The refusal for a [`FieldKind::Number`] buffer that is not a finite number.
///
/// Not built from [`forms::NUMBER_FORMS`] through [`refuse`], for
/// [`not_a_whole_number`]'s reason: the table's text is "a number", so
/// `refuse` would append "try a number" to a sentence that has just said "is
/// not a number". The table's own text still draws the panel, where "a number"
/// is the whole answer and reads correctly.
///
/// `pub(super)`: `ConfigPane::apply_typing`'s own number arm answers with
/// this, so the one sentence has one spelling.
///
/// `shown` is what may be printed: [`REDACTED`] rather than the value when the
/// field is a credential. Callers pass what [`refusal`] resolved.
pub(super) fn not_a_number(key: &str, shown: &str) -> Refusal {
    Refusal {
        text: format!("{key} is \"{shown}\", which is not a number shep accepts"),
    }
}

/// The refusal for a field whose own numeric range admits nothing.
///
/// Said once, ahead of the parse, because it is true of every value: a field
/// whose schema asks for a range with nothing in it cannot be answered, and
/// refusing `1` for being under the floor and `3` for being over the ceiling
/// bounces an operator between two sentences that cannot both be met. Both
/// number arms answer with this one sentence, so an integer's empty range and a
/// decimal's read the same to the operator.
///
/// `pub(super)`: [`numbers`] builds it twice, for the pair
/// [`crate::lookout::field::Bounds`] normalises for an integer and the pair it
/// keeps as written for a decimal.
pub(super) fn takes_no_value(key: &str) -> Refusal {
    Refusal {
        text: format!("{key} takes no value: its schema asks for a range with nothing in it"),
    }
}

/// The refusal for a value a schema's `enum` does not list, [`None`] when the
/// schema named no `enum` or `value` is one of the values it named.
///
/// The rule is shared because two doors meet it: [`items`] checks one element
/// of a list against its item schema's `enum`, and [`numbers`] checks a scalar
/// integer or decimal against the `enum` its own schema published. A set of
/// strings on a scalar is the case that is not here, because [`FieldKind::Choice`]
/// draws it as a cycle widget and nothing is typed to check.
///
/// The refusal lists the allowed set because an `enum` is the one rule an
/// operator can satisfy by copying from it.
///
/// `value` is what the buffer parsed to, not the buffer: an `enum` of numbers is
/// the numbers, and comparing a typed `"1"` against a published `1` is a
/// comparison of strings.
///
/// `pub(super)`: [`items`] and [`numbers`] both build their `enum` arm with it.
pub(super) fn enum_refusal(
    key: &str,
    bounds: &Bounds,
    value: &Value,
    shown: &str,
) -> Option<Refusal> {
    if bounds.choices.is_empty()
        || bounds
            .choices
            .iter()
            .any(|choice| choice_taken(choice, value))
    {
        return None;
    }
    let allowed: Vec<String> = bounds.choices.iter().map(choice_text).collect();
    Some(Refusal {
        text: format!(
            "{key} is \"{shown}\"; {key} takes one of {}",
            allowed.join(", ")
        ),
    })
}

/// Whether `value` is the `enum` member `choice` names.
///
/// Numbers compare by value, not by the spelling the schema used:
/// `serde_json` holds `1` and `1.0` as two different numbers, and JSON Schema
/// means one of them. Comparing the two `Value`s refused a `1` a dog published
/// as `enum: [1.0]`. Two integers are compared as `i64`, so a set of them is not
/// collapsed through an `f64` that cannot name every `i64` near its ends.
fn choice_taken(choice: &Value, value: &Value) -> bool {
    match (choice.as_i64(), value.as_i64()) {
        (Some(written), Some(typed)) => written == typed,
        _ => match (choice.as_f64(), value.as_f64()) {
            (Some(written), Some(typed)) => written == typed,
            _ => choice == value,
        },
    }
}

/// One `enum` member as the operator reads it: bare for a string, compact JSON
/// for anything else.
fn choice_text(choice: &Value) -> String {
    match choice {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// One refusal sentence, with what the field takes read off the same table
/// the explanation panel prints. Sharing the table is the point: a form
/// added to one is added to the other, and neither can name a spelling the
/// parser has stopped taking.
///
/// `shown` is what may be printed, which [`shown_value`] has already
/// resolved. Naming the parameter for the raw buffer invited a caller to
/// pass one.
fn refuse(key: &str, shown: &str, what: &str, forms: &[Form]) -> Refusal {
    let takes: Vec<&str> = forms.iter().map(|form| form.text).collect();
    Refusal {
        text: format!(
            "{key} is \"{shown}\", which is not {what} shep accepts; try {}",
            takes.join(", ")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::text_field;
    use super::forms::{DURATION_FORMS, DURATION_REFUSALS, MEMORY_FORMS};
    use super::*;
    use shep_core::values::UpDuration;
    /// The one rule both refusal doors share. `ConfigPane::apply_typing`
    /// builds its own sentence for an integer it cannot convert, so the
    /// redaction cannot live inside [`refusal`] alone.
    #[test]
    fn the_shown_value_of_a_secret_is_the_marker_and_never_the_buffer() {
        let mut field = text_field("token");
        assert_eq!(
            shown_value(field.secret, "sk_live_HUNTER2"),
            "sk_live_HUNTER2"
        );
        field.secret = true;
        assert_eq!(shown_value(field.secret, "sk_live_HUNTER2"), REDACTED);
        assert!(
            !not_a_whole_number(&field.key, shown_value(field.secret, "sk_live_HUNTER2"))
                .text
                .contains("HUNTER2")
        );
    }

    /// Driven through the same schema shape a real dog publishes:
    /// `shep-log-rotate` spells `max_age` as `Option<UpDuration>`, so the
    /// grammar sits two hops from the property, behind an `anyOf` and a
    /// `$ref`.
    ///
    /// `1d` is the natural spelling of a retention window and shep's
    /// grammar tops out at hours (`docs/specs/deferred.md`), so the panel
    /// must not offer it and the gate must refuse it.
    #[test]
    fn a_dogs_nullable_duration_refuses_a_day() {
        let field = duration_field();
        assert_eq!(
            field.value_kind,
            Some(ValueKind::UpDuration),
            "the $ref must resolve through the anyOf or this test proves nothing"
        );
        assert!(
            "1d".parse::<UpDuration>().is_err(),
            "1d must be outside the grammar or there is nothing to refuse"
        );
        let refused = refusal(&field, "1d").expect("1d is not a duration shep parses");
        assert_eq!(
            refused.text,
            "max_age is \"1d\", which is not a duration shep accepts; \
             try 500ms, 2s, 5m, 1h, a bare number is milliseconds"
        );
    }

    /// The passing case, which is what tells this gate apart from one that
    /// refuses everything.
    #[test]
    fn the_spellings_the_panel_offers_are_the_spellings_it_takes() {
        let field = duration_field();
        for accepted in DURATION_FORMS.iter().flat_map(|form| form.examples) {
            assert_eq!(
                refusal(&field, accepted),
                None,
                "{accepted} is printed as accepted and must be filed"
            );
        }
        for refused in DURATION_REFUSALS.iter().flat_map(|form| form.examples) {
            assert!(
                refusal(&field, refused).is_some(),
                "{refused} is printed as refused and must not be filed"
            );
        }
    }

    /// The size half, which had no test of its own here: it was reached
    /// only through `pane/edit.rs`'s `max_memory` case, so a change to
    /// that test's field would have taken this arm's coverage with it
    /// without failing anything.
    ///
    /// `10MB` is the spelling to refuse rather than a nonsense word.
    /// shep's grammar is `10M`, so `10MB` is what somebody writes who
    /// knows what they mean, and the sentence has to be the one that tells
    /// them the unit is a single letter.
    #[test]
    fn a_size_field_refuses_a_spelling_shep_does_not_take() {
        let mut field = text_field("max_size");
        field.value_kind = Some(ValueKind::MemSize);
        assert_eq!(
            refusal(&field, "10MB")
                .expect("10MB is not shep's spelling")
                .text,
            "max_size is \"10MB\", which is not a size shep accepts; \
             try 512M, 2G, a bare number is bytes"
        );
        for accepted in MEMORY_FORMS.iter().flat_map(|form| form.examples) {
            assert_eq!(
                refusal(&field, accepted),
                None,
                "{accepted} is printed as accepted and must be filed"
            );
        }
    }

    /// A secret's value must not reach the status bar, and both doors a
    /// schema opens onto this gate have to be shut: a `pattern`, and a
    /// `$ref` grammar. `ConfigPane::begin_typing` seeds a secret's editor
    /// empty and its row draws `<set>`, so a refusal echoing the buffer
    /// would be the one place in the pane a credential appears.
    ///
    /// Exact strings, both for the sentence and for the `Debug`, which is
    /// what IR-41 asks for on anything that can carry one.
    #[test]
    fn a_secrets_value_never_reaches_the_sentence_or_the_debug() {
        const CREDENTIAL: &str = "sk_live_HUNTER2";

        let mut pattern_door = text_field("token");
        pattern_door.secret = true;
        pattern_door.bounds.pattern = Some("^sk-".to_owned());
        let refused = refusal(&pattern_door, CREDENTIAL).expect("the pattern refuses it");
        assert_eq!(
            refused.text,
            "token is \"<set>\", which it does not take; try it matches ^sk-"
        );
        assert_eq!(
            format!("{refused:?}"),
            "Refusal { text: \"token is \\\"<set>\\\", which it does not take; try it matches ^sk-\" }"
        );

        let mut grammar_door = text_field("api_key");
        grammar_door.secret = true;
        grammar_door.value_kind = Some(ValueKind::UpDuration);
        let refused = refusal(&grammar_door, CREDENTIAL).expect("not a duration");
        assert!(!refused.text.contains(CREDENTIAL), "{}", refused.text);
        assert!(refused.text.contains("<set>"), "{}", refused.text);

        let mut integer_door = text_field("pin");
        integer_door.secret = true;
        integer_door.kind = FieldKind::Integer;
        let refused = refusal(&integer_door, CREDENTIAL).expect("not a number");
        assert_eq!(
            refused.text,
            "pin is \"<set>\", which is not a whole number shep accepts"
        );

        let mut bounded = text_field("pin");
        bounded.secret = true;
        bounded.kind = FieldKind::Integer;
        bounded.bounds.minimum = Some(1000);
        let refused = refusal(&bounded, "7").expect("below the floor");
        assert_eq!(refused.text, "pin is \"<set>\"; pin starts at 1000");

        // A decimal is the third door a schema opens onto this gate, and it
        // quotes the buffer the same way the other two do.
        let mut decimal = text_field("rate");
        decimal.secret = true;
        decimal.kind = FieldKind::Number;
        decimal.bounds.number_exclusive_minimum = Some(bound(0));
        let refused = refusal(&decimal, CREDENTIAL).expect("not a number");
        assert_eq!(
            refused.text,
            "rate is \"<set>\", which is not a number shep accepts"
        );
        let refused = refusal(&decimal, "0").expect("below the floor");
        assert_eq!(refused.text, "rate is \"<set>\"; rate must be above 0");
    }

    /// The control: a field that is not a credential still quotes what was
    /// typed, since redacting everything would pass the test above while
    /// making every refusal useless.
    #[test]
    fn an_ordinary_field_still_quotes_what_was_typed() {
        let mut field = text_field("region");
        field.bounds.pattern = Some("^eu-".to_owned());
        assert!(
            refusal(&field, "Europe")
                .expect("refused")
                .text
                .contains("Europe"),
        );
    }

    /// An empty buffer is `ConfigPane::apply_typing`'s unset case, not a
    /// value, and this gate must not take it over.
    #[test]
    fn an_empty_buffer_is_not_a_refusal() {
        assert_eq!(refusal(&duration_field(), ""), None);
    }

    /// Most fields are a plain string their schema says nothing about, and
    /// this gate is not shep's chance to invent a grammar for a dog's
    /// config. Nothing is refused there.
    #[test]
    fn a_field_with_no_grammar_and_no_pattern_refuses_nothing() {
        assert_eq!(refusal(&text_field("cwd"), "/srv/web"), None);
        assert_eq!(refusal(&text_field("cwd"), "!!! anything"), None);
    }

    /// A [`ValueKind::UpDuration`] field built the way a dog's schema
    /// spells an optional one: `anyOf: [{$ref}, {type: null}]`, with the
    /// grammar in `$defs`.
    fn duration_field() -> Field {
        let schema = serde_json::json!({
            "$defs": {
                "UpDuration": { "type": "string", "pattern": r"^\d+(ms|h|m|s)?$" },
            },
            "properties": {
                "max_age": {
                    "anyOf": [{ "$ref": "#/$defs/UpDuration" }, { "type": "null" }],
                    "description": "Also rotate this long after the last rotation.",
                },
            },
        });
        crate::lookout::pane::ConfigPane::dog("log-rotate".to_owned(), None, schema, String::new())
            .fields()
            .by_key("max_age")
            .expect("the schema declares max_age")
            .clone()
    }

    /// A bound written the way [`crate::lookout::field::Bounds`] keeps one.
    fn bound(value: i64) -> serde_json::Number {
        serde_json::json!(value)
            .as_number()
            .expect("a number")
            .clone()
    }
}
