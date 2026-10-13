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

#[cfg(test)]
mod fixtures;
mod forms;

pub use forms::bullets;

use serde_json::Value;
use shep_core::values::{MemSize, UpDuration};

use super::field::{Bounds, Field, FieldKind, ListItem, ValueKind};
use forms::{DURATION_FORMS, Form, MEMORY_FORMS};

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
/// Three checks, and a field reaches at most one of them:
///
/// - A [`FieldKind::Integer`] buffer must parse as an `i64` and sit inside
///   whatever `minimum` and `maximum` the schema published.
/// - A string field naming one of shep-core's own grammars goes through
///   that type's `FromStr`, which is the enforcer itself rather than a
///   second opinion about it.
/// - Any other string field with a `pattern` is matched against it. This
///   is the only door a third-party dog's own grammar has, since shep has
///   no parser for a type it has never heard of.
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

/// Why one typed element of a list field was not filed, in the operator's
/// words.
///
/// The rules are the ones the item schema published, which is a different
/// schema from the one the dog published about the array: `minLength`,
/// `pattern` and `enum` for a string list, the schema's floor and ceiling
/// for an integer one, and `enum` for both. An element with no rule on it
/// is accepted, exactly as a scalar field its schema says nothing about is:
/// shep is not the authority on a dog's config beyond what that dog
/// published.
///
/// `secret` decides what the sentence may print, resolved here rather than
/// per rule so a rule added later cannot echo a credential by forgetting to
/// ask. A list the schema marks secret is rare rather than impossible: a
/// dog's `Vec<String>` of tokens publishes `x-shep-secret` on its items.
///
/// Fails open on a `pattern` that does not compile, for [`refusal`]'s
/// reason.
///
/// The empty buffer never reaches here: it is nothing to write, which is
/// [`super::pane::ListPane::apply_typing`]'s own case.
#[must_use]
pub fn item_refusal(
    key: &str,
    item: ListItem,
    bounds: &Bounds,
    typed: &str,
    secret: bool,
) -> Option<Refusal> {
    let shown = shown_value(secret, typed);
    let value = match item {
        ListItem::Integer => match integer_of(key, bounds, typed, shown) {
            Ok(number) => {
                if let Some(refused) = range_refusal(key, bounds, shown, number) {
                    return Some(refused);
                }
                Value::from(number)
            }
            Err(refused) => return Some(refused),
        },
        ListItem::Text => {
            if let Some(refused) = min_length_refusal(key, bounds, typed, shown) {
                return Some(refused);
            }
            if let Some(refused) =
                pattern_refusal(key, bounds.pattern.as_deref(), &[], typed, shown)
            {
                return Some(refused);
            }
            Value::String(typed.to_owned())
        }
    };
    enum_refusal(key, bounds, &value, shown)
}

/// [`item_refusal`]'s `minLength` arm. The count is of characters, so an
/// element with one accented letter is one character long rather than two.
fn min_length_refusal(key: &str, bounds: &Bounds, typed: &str, shown: &str) -> Option<Refusal> {
    let least = bounds.min_length?;
    (typed.chars().count() < least).then(|| Refusal {
        text: format!("{key} is \"{shown}\"; {key} needs at least {least} characters"),
    })
}

/// [`item_refusal`]'s `enum` arm: the element as the JSON value it parsed
/// to, against the values the schema wrote.
///
/// The refusal lists the allowed set because an `enum` is the one rule an
/// operator can satisfy by copying from it.
fn enum_refusal(key: &str, bounds: &Bounds, value: &Value, shown: &str) -> Option<Refusal> {
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
/// `serde_json` holds `1` and `1.0` as two different numbers, and JSON
/// Schema means one of them. Comparing the two `Value`s refused a `1` a dog
/// published as `enum: [1.0]`. Two integers are compared as `i64`, so a set
/// of them is not collapsed through an `f64` that cannot name every `i64`
/// near its ends.
fn choice_taken(choice: &Value, value: &Value) -> bool {
    match (choice.as_i64(), value.as_i64()) {
        (Some(written), Some(typed)) => written == typed,
        _ => match (choice.as_f64(), value.as_f64()) {
            (Some(written), Some(typed)) => written == typed,
            _ => choice == value,
        },
    }
}

/// One `enum` member as the operator reads it: bare for a string, compact
/// JSON for anything else.
fn choice_text(choice: &Value) -> String {
    match choice {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// [`refusal`] for a [`FieldKind::Integer`] field: the empty range, the
/// parse, then the schema's own floor and ceiling.
///
/// `shown` is what may be printed, which is `typed` for an ordinary field
/// and [`REDACTED`] for a secret one. `typed` is still what gets parsed.
fn integer_refusal(key: &str, bounds: &Bounds, typed: &str, shown: &str) -> Option<Refusal> {
    match integer_of(key, bounds, typed, shown) {
        Ok(number) => range_refusal(key, bounds, shown, number),
        Err(refused) => Some(refused),
    }
}

/// `typed` as the whole number it names, or the sentence saying it names
/// none.
///
/// The empty range is checked ahead of the parse, because it is true of
/// every value: a field whose schema admits no integer cannot be answered,
/// and saying "starts at 3" to somebody who then types 3 and is told it
/// "stops at 1" explains nothing.
fn integer_of(key: &str, bounds: &Bounds, typed: &str, shown: &str) -> Result<i64, Refusal> {
    if bounds.unsatisfiable {
        return Err(Refusal {
            text: format!("{key} takes no value: its schema asks for a range with nothing in it"),
        });
    }
    typed
        .parse::<i64>()
        .map_err(|_| not_a_whole_number(key, shown))
}

/// The floor and ceiling `bounds` sets on an integer already parsed.
fn range_refusal(key: &str, bounds: &Bounds, shown: &str, number: i64) -> Option<Refusal> {
    match (bounds.minimum, bounds.maximum) {
        (Some(min), _) if number < min => Some(Refusal {
            text: format!("{key} is \"{shown}\"; {key} starts at {min}"),
        }),
        (_, Some(max)) if number > max => Some(Refusal {
            text: format!("{key} is \"{shown}\"; {key} stops at {max}"),
        }),
        _ => None,
    }
}

/// [`refusal`] for a string field whose schema published a `pattern` and
/// whose type shep does not know: the dog's own grammar, enforced as the
/// dog wrote it.
///
/// The field's own `accepts` is what the refusal names when it has any,
/// since that is the spelling its author wrote for an operator. The raw
/// pattern is the fallback, which is worse copy than a sentence and better
/// than the silence it replaced. A list's item schema has no `accepts` of
/// its own, so [`item_refusal`] passes none.
///
/// Fails open on a `pattern` that does not compile: `.ok()?`.
fn pattern_refusal(
    key: &str,
    pattern: Option<&str>,
    accepts: &[String],
    typed: &str,
    shown: &str,
) -> Option<Refusal> {
    let compiled = regex::Regex::new(pattern?).ok()?;
    if compiled.is_match(typed) {
        return None;
    }
    let takes = if accepts.is_empty() {
        format!("it matches {}", pattern?)
    } else {
        accepts.join(", ")
    };
    Some(Refusal {
        text: format!("{key} is \"{shown}\", which it does not take; try {takes}"),
    })
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

    /// A range with nothing in it is said once, rather than as a floor and
    /// a ceiling the operator is bounced between. A dog writing
    /// `range(min = 3, max = 1)` is the realistic way to get one.
    #[test]
    fn a_range_with_nothing_in_it_says_so() {
        let mut field = text_field("workers");
        field.kind = FieldKind::Integer;
        field.bounds.minimum = Some(3);
        field.bounds.maximum = Some(1);
        field.bounds.unsatisfiable = true;
        for typed in ["1", "2", "3", "not a number"] {
            assert_eq!(
                refusal(&field, typed).expect("nothing is acceptable").text,
                "workers takes no value: its schema asks for a range with nothing in it",
                "{typed}"
            );
        }
    }

    /// An empty buffer is `ConfigPane::apply_typing`'s unset case, not a
    /// value, and this gate must not take it over.
    #[test]
    fn an_empty_buffer_is_not_a_refusal() {
        assert_eq!(refusal(&duration_field(), ""), None);
    }

    /// A dog's own grammar, which shep has no parser for: the `pattern` is
    /// the only thing that can refuse it, and the sentence names the
    /// field's own `accepts` rather than a regex when it has any.
    #[test]
    fn a_dogs_own_pattern_refuses_what_it_does_not_match() {
        let mut field = text_field("region");
        field.bounds.pattern = Some("^[a-z]{2}-[a-z]+-[0-9]$".to_owned());
        assert_eq!(
            refusal(&field, "eu-west-1"),
            None,
            "the pattern's own shape"
        );
        let refused = refusal(&field, "Europe").expect("Europe does not match");
        assert_eq!(
            refused.text,
            "region is \"Europe\", which it does not take; try it matches ^[a-z]{2}-[a-z]+-[0-9]$"
        );

        field.accepts = vec!["a region like eu-west-1".to_owned()];
        let refused = refusal(&field, "Europe").expect("still refused");
        assert_eq!(
            refused.text,
            "region is \"Europe\", which it does not take; try a region like eu-west-1",
            "the author's own words win over the regex"
        );
    }

    /// A dog that ships a regex that does not compile has a bug in its
    /// schema. Refusing every value for that field would lock an operator
    /// out of a setting over somebody else's typo, so the gate fails open.
    #[test]
    fn a_pattern_that_does_not_compile_refuses_nothing() {
        // Spelled in two pieces because `clippy::invalid_regex` reads a
        // literal handed to `Regex::new` and refuses to compile the file
        // over the very thing this test needs to be broken.
        let broken = ["^[", "a-z"].concat();
        assert!(
            regex::Regex::new(&broken).is_err(),
            "the fixture must be a pattern regex really refuses"
        );
        let mut field = text_field("region");
        field.bounds.pattern = Some(broken);
        assert_eq!(refusal(&field, "anything at all"), None);
    }

    /// `keep = 0` deletes every rotation the moment it is made, and
    /// `shep-log-rotate` publishes `minimum: 1` in its schema saying so.
    /// Its comment there names this exact failure: without the floor the
    /// pane offers a value the dog refuses on its next tick, in a file the
    /// operator has already saved and moved on from.
    #[test]
    fn an_integers_schema_floor_and_ceiling_are_enforced() {
        let mut field = text_field("keep");
        field.kind = FieldKind::Integer;
        field.bounds.minimum = Some(1);
        field.bounds.maximum = Some(64);
        assert_eq!(refusal(&field, "1"), None, "the floor itself is allowed");
        assert_eq!(refusal(&field, "64"), None, "the ceiling itself is allowed");
        // Whole sentences, because a fragment cannot see that the value
        // beside it was rendered bare while every other refusal quoted it.
        let refused = refusal(&field, "0").expect("0 is below the floor");
        assert_eq!(refused.text, "keep is \"0\"; keep starts at 1");
        let refused = refusal(&field, "65").expect("65 is above the ceiling");
        assert_eq!(refused.text, "keep is \"65\"; keep stops at 64");
    }

    /// An integer with no bounds still has to be an integer, and the
    /// sentence says so rather than leaving the editor open in silence.
    #[test]
    fn an_integer_field_refuses_text_out_loud() {
        let mut field = text_field("max_restarts");
        field.kind = FieldKind::Integer;
        let refused = refusal(&field, "lots").expect("lots is not a number");
        // The whole sentence, because a `contains("whole number")` passed
        // one that went on to say "try a whole number".
        assert_eq!(
            refused.text,
            "max_restarts is \"lots\", which is not a whole number shep accepts"
        );
    }

    /// Most fields are a plain string their schema says nothing about, and
    /// this gate is not shep's chance to invent a grammar for a dog's
    /// config. Nothing is refused there.
    #[test]
    fn a_field_with_no_grammar_and_no_pattern_refuses_nothing() {
        assert_eq!(refusal(&text_field("cwd"), "/srv/web"), None);
        assert_eq!(refusal(&text_field("cwd"), "!!! anything"), None);
    }

    /// Every rule an item schema can publish, with the value it takes and
    /// the one it turns down, and the whole sentence for the refused one.
    ///
    /// `minLength` is checked at exactly its bound, because a rule that
    /// refused its own minimum would be a field nothing can be set to.
    #[test]
    fn every_item_rule_refuses_one_element_and_takes_the_next() {
        let min_length = Bounds {
            min_length: Some(3),
            ..Default::default()
        };
        let pattern = Bounds {
            pattern: Some("^[a-z]+$".to_owned()),
            ..Default::default()
        };
        let words = Bounds {
            choices: vec![serde_json::json!("ask"), serde_json::json!("auto")],
            ..Default::default()
        };
        let numbers = Bounds {
            choices: vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3),
            ],
            ..Default::default()
        };
        let range = Bounds {
            minimum: Some(1),
            maximum: Some(64),
            ..Default::default()
        };
        let cases = [
            (
                "minLength",
                ListItem::Text,
                &min_length,
                "abc",
                "ab",
                "tags is \"ab\"; tags needs at least 3 characters",
            ),
            (
                "pattern",
                ListItem::Text,
                &pattern,
                "abc",
                "Ab1",
                "tags is \"Ab1\", which it does not take; try it matches ^[a-z]+$",
            ),
            (
                "enum",
                ListItem::Text,
                &words,
                "auto",
                "never",
                "tags is \"never\"; tags takes one of ask, auto",
            ),
            (
                "integer enum",
                ListItem::Integer,
                &numbers,
                "2",
                "4",
                "tags is \"4\"; tags takes one of 1, 2, 3",
            ),
            (
                "minimum",
                ListItem::Integer,
                &range,
                "1",
                "0",
                "tags is \"0\"; tags starts at 1",
            ),
            (
                "maximum",
                ListItem::Integer,
                &range,
                "64",
                "65",
                "tags is \"65\"; tags stops at 64",
            ),
        ];
        for (rule, item, bounds, accepted, refused, sentence) in cases {
            assert_eq!(
                item_refusal("tags", item, bounds, accepted, false),
                None,
                "{rule}: {accepted} is the value the rule takes"
            );
            let refused = item_refusal("tags", item, bounds, refused, false).expect(rule);
            assert_eq!(refused.text, sentence, "{rule}");
        }
    }

    /// An `enum` member a schema wrote with a decimal point is the number
    /// the pane parsed, and a `Value` comparison says otherwise:
    /// `serde_json` keeps `1` and `1.0` as two numbers, and JSON Schema
    /// means one of them. A dog publishing `enum: [1.0, 2.0]` for a
    /// `Vec<i64>` had its own values refused.
    #[test]
    fn an_enum_number_written_with_a_decimal_point_still_takes_the_integer() {
        let bounds = Bounds {
            choices: vec![serde_json::json!(1.0), serde_json::json!(2.0)],
            ..Default::default()
        };
        for typed in ["1", "2"] {
            assert_eq!(
                item_refusal("levels", ListItem::Integer, &bounds, typed, false),
                None,
                "{typed} is the value the schema wrote"
            );
        }
        let refused = item_refusal("levels", ListItem::Integer, &bounds, "3", false)
            .expect("3 is not in the set");
        assert_eq!(
            refused.text,
            "levels is \"3\"; levels takes one of 1.0, 2.0"
        );

        // The other direction holds: a set holding a fraction does not take
        // the whole number beside it.
        let halves = Bounds {
            choices: vec![serde_json::json!(1.5)],
            ..Default::default()
        };
        let refused =
            item_refusal("levels", ListItem::Integer, &halves, "1", false).expect("1 is not 1.5");
        assert_eq!(refused.text, "levels is \"1\"; levels takes one of 1.5");
    }

    /// The one thing an integer list refuses even with no bounds on its
    /// items, and the sentence the scalar editor already uses for it.
    #[test]
    fn an_integer_element_that_is_not_a_number_is_refused_out_loud() {
        let bounds = Bounds::default();
        let refused = item_refusal("stop_exit_codes", ListItem::Integer, &bounds, "lots", false)
            .expect("lots is not a number");
        assert_eq!(
            refused.text,
            "stop_exit_codes is \"lots\", which is not a whole number shep accepts"
        );
        assert_eq!(
            item_refusal("stop_exit_codes", ListItem::Integer, &bounds, "-1", false),
            None,
            "a negative exit code is a whole number"
        );
    }

    /// A list whose items are a plain string, which is every list the
    /// Flockfile schema publishes today. Nothing is refused, and the gate
    /// is not shep's chance to invent a grammar for a dog's array.
    #[test]
    fn a_list_with_no_item_rules_refuses_nothing() {
        let bounds = Bounds::default();
        for typed in ["a", "!!! anything", ""] {
            assert_eq!(
                item_refusal("args", ListItem::Text, &bounds, typed, false),
                None,
                "{typed}"
            );
        }
    }

    /// The count is of characters, not bytes. `"ab\u{00e9}"` is four bytes
    /// and three characters, so a byte count would have taken it and a
    /// character count refuses it.
    #[test]
    fn a_min_length_counts_characters_and_not_bytes() {
        let bounds = Bounds {
            min_length: Some(4),
            ..Default::default()
        };
        assert_eq!(
            item_refusal("tags", ListItem::Text, &bounds, "abc\u{00e9}", false),
            None,
            "four characters, five bytes"
        );
        let refused = item_refusal("tags", ListItem::Text, &bounds, "ab\u{00e9}", false)
            .expect("three characters is under four");
        assert_eq!(
            refused.text,
            "tags is \"ab\u{00e9}\"; tags needs at least 4 characters"
        );
    }

    /// A dog that ships a broken regex in its item schema has a bug in its
    /// schema, and the list editor fails open on it for [`refusal`]'s
    /// reason: refusing every element would lock an operator out of a
    /// setting over somebody else's typo.
    #[test]
    fn a_list_pattern_that_does_not_compile_refuses_nothing() {
        let broken = ["^[", "a-z"].concat();
        assert!(
            regex::Regex::new(&broken).is_err(),
            "the fixture must be a pattern regex really refuses"
        );
        let bounds = Bounds {
            pattern: Some(broken),
            ..Default::default()
        };
        assert_eq!(
            item_refusal("tags", ListItem::Text, &bounds, "ab", false),
            None,
            "a pattern that does not compile bounds nothing"
        );
    }

    /// A list of credentials reaches a refusal the same two doors a scalar
    /// does, and neither may print the element: the list sub-screen draws
    /// what is typed, and this sentence is the one place a value typed into
    /// it would be repeated.
    #[test]
    fn a_secret_lists_refusal_names_the_marker_and_never_the_element() {
        const CREDENTIAL: &str = "tok_HUNTER2";
        let pattern = Bounds {
            pattern: Some("^sk-".to_owned()),
            ..Default::default()
        };
        let refused = item_refusal("tokens", ListItem::Text, &pattern, CREDENTIAL, true)
            .expect("the pattern refuses it");
        assert_eq!(
            refused.text,
            "tokens is \"<set>\", which it does not take; try it matches ^sk-"
        );

        let min_length = Bounds {
            min_length: Some(32),
            ..Default::default()
        };
        let refused = item_refusal("tokens", ListItem::Text, &min_length, CREDENTIAL, true)
            .expect("too short");
        assert!(!refused.text.contains(CREDENTIAL), "{}", refused.text);
        assert!(refused.text.contains("<set>"), "{}", refused.text);

        let choices = Bounds {
            choices: vec![serde_json::json!("ask"), serde_json::json!("auto")],
            ..Default::default()
        };
        let refused = item_refusal("tokens", ListItem::Text, &choices, CREDENTIAL, true)
            .expect("not a member");
        assert!(!refused.text.contains(CREDENTIAL), "{}", refused.text);

        let range = Bounds {
            minimum: Some(1000),
            ..Default::default()
        };
        let refused =
            item_refusal("pins", ListItem::Integer, &range, "7", true).expect("below the floor");
        assert_eq!(refused.text, "pins is \"<set>\"; pins starts at 1000");
    }

    /// The control on the test above: an element of a list that is not a
    /// credential is quoted back, since redacting everything would pass
    /// that test while making every refusal useless.
    #[test]
    fn an_ordinary_element_is_still_quoted_back() {
        let bounds = Bounds {
            pattern: Some("^eu-".to_owned()),
            ..Default::default()
        };
        assert!(
            item_refusal("regions", ListItem::Text, &bounds, "Europe", false)
                .expect("refused")
                .text
                .contains("Europe"),
        );
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
}
