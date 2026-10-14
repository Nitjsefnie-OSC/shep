//! What a schema says a value must satisfy, beyond its type.
//!
//! Separate from [`super::schema`] because the grammar is its own: JSON
//! Schema writes `minimum` and `maximum` as arbitrary numbers, and a
//! `Field` needs a pair of `i64` bounds for an integer, the same bounds as
//! written for a decimal, and an answer for a range that holds nothing.
//! Normalising one into the other is the whole job here.
//!
//! An array's `items` schema is read the same way and hung under [`Bounds::items`],
//! so a list element is checked against what the dog published about an
//! element rather than about the array.

use serde_json::{Map, Number, Value};

use super::schema::resolved;

/// The bounds a property's schema publishes beyond its type, for the
/// keywords this pane can actually check a keystroke against: `pattern`,
/// `minLength`, `enum`, and the numeric range, once normalised for an integer
/// and once as written for a decimal.
///
/// Read off the resolved schema, so a `$ref` into `$defs` and an
/// `anyOf: [T, null]` are both followed first: `max_age` on a dog that
/// spells it `Option<UpDuration>` carries its pattern two hops away from
/// the property itself.
///
/// Empty for a field whose schema says nothing machine-checkable, which is
/// most of them. A bound this type does not carry is a bound
/// [`super::super::validation::refusal`] cannot enforce, and the pane
/// files the value: shep is not the authority on a dog's own config.
///
/// `Debug` is derived (IR-41): a schema's bounds describe a value without
/// carrying one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bounds {
    /// `pattern`, as the schema wrote it. A dog's own grammar reaches the
    /// pane only through this, since shep has no `FromStr` for a type it
    /// has never heard of.
    pub pattern: Option<String>,
    /// `minLength`, counted in characters. Only an array's item schema is
    /// checked against it today: [`super::super::validation::refusal`] reads
    /// a scalar's `minLength` and enforces nothing with it.
    pub min_length: Option<usize>,
    /// `minimum`, for a [`super::FieldKind::Integer`] field, normalised
    /// to the smallest `i64` the schema accepts. shep-log-rotate's `keep`
    /// publishes one for exactly this reason: without it the pane offers a
    /// `keep = 0` the dog refuses on its next tick, in a file the operator
    /// has already saved and moved on from.
    pub minimum: Option<i64>,
    /// `maximum`, normalised to the largest `i64` the schema accepts.
    pub maximum: Option<i64>,
    /// `minimum`, as the schema wrote it, for a [`super::FieldKind::Number`]
    /// field. Kept beside [`Self::minimum`] rather than in place of it: a
    /// decimal field's floor is a decimal, and rounding `0.5` up to `1` would
    /// refuse the value the schema itself names.
    ///
    /// A `Number` rather than an `f64` for the same reason `choices` holds
    /// JSON values: it keeps `Bounds` an `Eq` type, and it renders back to the
    /// operator as the schema spelled it.
    pub number_minimum: Option<Number>,
    /// `maximum`, as the schema wrote it, for a [`super::FieldKind::Number`]
    /// field.
    pub number_maximum: Option<Number>,
    /// `exclusiveMinimum`, as written. JSON Schema's strict form of a floor,
    /// and the one `shep-kelpie` publishes on `budget.per_item_usd`. Read for
    /// a decimal field only: an integer's [`Self::minimum`] already carries
    /// the rounding an exclusive bound would need.
    pub number_exclusive_minimum: Option<Number>,
    /// `exclusiveMaximum`, as written. The strict ceiling, read for a decimal
    /// field only, for [`Self::number_exclusive_minimum`]'s reason.
    pub number_exclusive_maximum: Option<Number>,
    /// `enum`, kept as JSON values rather than strings so an integer list's
    /// `enum` of numbers compares against a parsed element. Empty is the
    /// common case and means the schema named no `enum`.
    ///
    /// A scalar's `enum` reaches the operator as a cycle widget
    /// ([`super::FieldKind::Choice`]) rather than through here, so only a
    /// list's item `enum` is checked against these values.
    pub choices: Vec<Value>,
    /// Whether the schema's own numeric range admits no integer at all.
    ///
    /// A dog that writes `range(min = 3, max = 1)` publishes a field
    /// nothing can set. Without this the pane refuses `1` for being under
    /// the floor and `3` for being over the ceiling, one sentence at a
    /// time, and never says that no answer exists.
    pub unsatisfiable: bool,
    /// For an array field, what its `items` schema publishes, followed
    /// through a `$ref` and an `anyOf: [T, null]` exactly as a scalar's
    /// bounds are. [`None`] for a field that is not an array.
    pub items: Option<Box<Bounds>>,
}

/// The [`Bounds`] the resolved schema publishes.
///
/// A numeric bound is read as an `f64` and normalised to the integer it
/// actually forbids, because JSON Schema's bounds are inclusive: a
/// `minimum` rounds UP and a `maximum` rounds DOWN, so `minimum: 0.5`
/// forbids `0` and `maximum: 2.5` forbids `3`. The same four keywords are
/// kept unnormalised beside them for a [`super::FieldKind::Number`] field,
/// which is held to what the schema wrote rather than to a rounding of it.
///
/// Reading them with `as_i64` instead dropped both, and a dropped bound is
/// an accepted keystroke: the pane filed a `0` its own schema forbade.
/// Reading them as `f64` also keeps a bound written as `1e30`, which no
/// `i64` satisfies, apart from one written as `1`.
pub(super) fn bounds_of(schema: &Value, defs: &Map<String, Value>) -> Bounds {
    bounds_opening(schema, defs, &mut Vec::new())
}

/// [`bounds_of`]'s walk. `open` holds the schemas on the current path, by
/// identity, each one twice: as written, and as [`resolved`] reads it.
///
/// An array whose `items` names its own type, which `schemars` emits for a
/// recursive one, publishes no item bounds rather than recursing until the
/// stack is gone. The raw form is what makes that hold for a `$defs` entry
/// that is only an alias: `B` resolves to `A`, so a path of resolved values
/// alone never meets `B`, and the walk resolves `B` to `A`, pushes `A`, and
/// finds `B` again for as long as it has stack.
fn bounds_opening<'a>(
    schema: &'a Value,
    defs: &'a Map<String, Value>,
    open: &mut Vec<&'a Value>,
) -> Bounds {
    let depth = open.len();
    let written = schema;
    let schema = resolved(schema, defs);
    open.push(written);
    open.push(schema);
    let items = schema
        .get("items")
        .filter(|items| items.is_object())
        .map(|items| resolved(items, defs))
        .filter(|item| !open.iter().any(|seen| core::ptr::eq(*seen, *item)))
        .map(|item| Box::new(bounds_opening(item, defs, open)));
    open.truncate(depth);
    let minimum = schema.get("minimum").and_then(Value::as_f64).map(f64::ceil);
    let maximum = schema
        .get("maximum")
        .and_then(Value::as_f64)
        .map(f64::floor);
    let (floor, ceiling) = (minimum.and_then(as_bound), maximum.and_then(as_bound));
    // The same keywords, unnormalised, for a field whose values are decimals.
    // A bound the schema wrote as a number is read as one here, so `minimum: 0`
    // and `minimum: 0.5` reach the gate as what they say, and a bound that is
    // not a number at all bounds nothing.
    let number_bound = |keyword: &str| schema.get(keyword).and_then(Value::as_number).cloned();
    Bounds {
        pattern: schema
            .get("pattern")
            .and_then(Value::as_str)
            .map(str::to_owned),
        // A `minLength` that is not a whole non-negative number is not a
        // `minLength` JSON Schema allows, and dropping it fails open.
        min_length: schema
            .get("minLength")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok()),
        minimum: floor,
        maximum: ceiling,
        number_minimum: number_bound("minimum"),
        number_maximum: number_bound("maximum"),
        number_exclusive_minimum: number_bound("exclusiveMinimum"),
        number_exclusive_maximum: number_bound("exclusiveMaximum"),
        choices: schema
            .get("enum")
            .and_then(Value::as_array)
            .map(|values| values.to_vec())
            .unwrap_or_default(),
        // A bound past an `i64`'s reach is impossible in one direction and
        // says nothing in the other: a `minimum` above `i64::MAX` admits
        // nothing, while one below `i64::MIN` admits every integer there
        // is, which is what `as_bound` already answered `None` for.
        unsatisfiable: minimum.is_some_and(|value| value > i64::MAX as f64)
            || maximum.is_some_and(|value| value < i64::MIN as f64)
            || matches!((floor, ceiling), (Some(low), Some(high)) if low > high),
        items,
    }
}

/// `value` as the `i64` bound it names, or [`None`] when it sits outside
/// what an `i64` can hold and so bounds nothing this pane can compare.
///
/// The comparison is in `f64`, which cannot name every `i64` exactly near
/// its ends. A bound within a few units of `i64::MAX` therefore saturates
/// to it rather than being refused, which is the right answer for a field
/// no operator is typing sixteen digits into.
fn as_bound(value: f64) -> Option<i64> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "guarded by the range check, and a float-to-int cast saturates"
    )]
    (value >= i64::MIN as f64 && value <= i64::MAX as f64).then_some(value as i64)
}

#[cfg(test)]
mod tests {
    use super::super::FieldSet;
    use super::super::fixtures::{props, real_field_set};
    use super::*;
    use serde_json::json;

    /// Every hop [`resolved`] follows, because a reader that followed one
    /// and not the other reports a nullable field's bounds as absent, and
    /// nothing downstream can tell that from a schema that published none.
    ///
    /// Three spellings of the same `pattern`: inline on the property, one
    /// `$ref` away, and behind the `anyOf: [{$ref}, {type: null}]` that a
    /// dog writes for an `Option<UpDuration>`. That third one is the shape
    /// `shep-log-rotate` publishes, so it is the shape the gate in
    /// [`super::super::validation::refusal`] actually meets.
    #[test]
    fn a_pattern_is_read_through_every_hop_a_schema_can_put_it_behind() {
        let defs = props(json!({
            "UpDuration": { "type": "string", "pattern": r"^\d+(ms|h|m|s)?$" },
        }));
        let p = props(json!({
            "inline": { "type": "string", "pattern": r"^\d+(ms|h|m|s)?$" },
            "one_hop": { "$ref": "#/$defs/UpDuration" },
            "two_hops": { "anyOf": [{ "$ref": "#/$defs/UpDuration" }, { "type": "null" }] },
            "plain": { "type": "string" },
        }));
        let set = FieldSet::from_properties(&p, &defs, &[]);
        for key in ["inline", "one_hop", "two_hops"] {
            assert_eq!(
                set.by_key(key).unwrap().bounds.pattern.as_deref(),
                Some(r"^\d+(ms|h|m|s)?$"),
                "{key}"
            );
        }
        // The negative control: bounds that came back non-empty for a
        // schema publishing none would pass every assertion above.
        assert_eq!(set.by_key("plain").unwrap().bounds, Bounds::default());
    }

    /// The numeric half, and the reason it is here rather than only in the
    /// gate's own tests: `shep-log-rotate` publishes `minimum: 1` on
    /// `keep`, so the floor an operator is held to is one this reader has
    /// to lift off somebody else's schema.
    ///
    /// A fractional bound is normalised to the integer it forbids, not
    /// dropped: JSON Schema's bounds are inclusive, so a `minimum` rounds
    /// up and a `maximum` rounds down. Dropping them accepted a `0` under
    /// a `minimum` of `0.5`, and the pane filed it.
    #[test]
    fn integer_bounds_are_read_and_a_fractional_one_is_normalised() {
        let p = props(json!({
            "keep": { "type": "integer", "minimum": 1 },
            "ratio": { "type": "integer", "minimum": 0.5, "maximum": 2.5 },
            "both": { "type": "integer", "minimum": -3, "maximum": 64 },
            "plain": { "type": "integer" },
        }));
        let set = FieldSet::from_properties(&p, &Default::default(), &[]);
        assert_eq!(set.by_key("keep").unwrap().bounds.minimum, Some(1));
        assert_eq!(set.by_key("both").unwrap().bounds.minimum, Some(-3));
        assert_eq!(set.by_key("both").unwrap().bounds.maximum, Some(64));
        let ratio = &set.by_key("ratio").unwrap().bounds;
        assert_eq!(ratio.minimum, Some(1), "0.5 rounds up, so 0 is refused");
        assert_eq!(ratio.maximum, Some(2), "2.5 rounds down, so 3 is refused");
        assert!(!ratio.unsatisfiable);
        assert_eq!(set.by_key("plain").unwrap().bounds, Bounds::default());
    }

    /// The bounds a [`super::FieldKind::Number`] field is held to, kept as the
    /// schema wrote them. A decimal field's floor is a decimal: normalising
    /// `minimum: 0.5` up to `1` would refuse `0.5`, the value the schema
    /// names, and `exclusiveMinimum: 0` is the spelling `shep-kelpie`'s
    /// `budget.per_item_usd` publishes, which the integer pair does not read
    /// at all.
    #[test]
    fn a_number_fields_bounds_are_kept_as_the_schema_wrote_them() {
        let p = props(json!({
            "per_item_usd": { "type": "number", "format": "double", "exclusiveMinimum": 0 },
            "ratio": { "type": "number", "minimum": 0.5, "maximum": 2.5 },
            "below": { "type": "number", "exclusiveMaximum": 10 },
            "plain": { "type": "number" },
        }));
        let set = FieldSet::from_properties(&p, &Default::default(), &[]);
        let bounds = |key: &str| set.by_key(key).expect(key).bounds.clone();
        let bound = |value: Value| value.as_number().expect("a number").clone();
        assert_eq!(
            bounds("per_item_usd").number_exclusive_minimum,
            Some(bound(json!(0)))
        );
        assert_eq!(
            (
                bounds("ratio").number_minimum,
                bounds("ratio").number_maximum
            ),
            (Some(bound(json!(0.5))), Some(bound(json!(2.5)))),
            "a decimal bound is not rounded"
        );
        assert_eq!(
            bounds("below").number_exclusive_maximum,
            Some(bound(json!(10)))
        );
        assert_eq!(
            bounds("ratio"),
            Bounds {
                number_minimum: Some(bound(json!(0.5))),
                number_maximum: Some(bound(json!(2.5))),
                // The integer pair sits beside these and is what an
                // [`super::FieldKind::Integer`] field is held to.
                minimum: Some(1),
                maximum: Some(2),
                ..Default::default()
            },
            "both halves are read, and neither replaces the other"
        );
        assert_eq!(
            bounds("plain"),
            Bounds::default(),
            "no bound, nothing to check"
        );
    }

    /// A bound the schema did not write as a number bounds nothing, the same
    /// fail-open answer a `pattern` that does not compile gets.
    #[test]
    fn a_numeric_bound_that_is_not_a_number_is_read_as_no_bound() {
        let p = props(json!({
            "ratio": { "type": "number", "minimum": "0.5", "maximum": null },
        }));
        let set = FieldSet::from_properties(&p, &Default::default(), &[]);
        let bounds = &set.by_key("ratio").expect("ratio").bounds;
        assert_eq!(bounds.number_minimum, None);
        assert_eq!(bounds.number_maximum, None);
    }

    /// A range no integer satisfies, by each of the three routes into one.
    /// The pane says so once rather than bouncing an operator between a
    /// floor and a ceiling that cannot both be met.
    #[test]
    fn a_range_with_nothing_in_it_is_recorded_as_such() {
        let p = props(json!({
            "inverted": { "type": "integer", "minimum": 3, "maximum": 1 },
            "inverted_after_rounding": {
                "type": "integer", "minimum": 1.2, "maximum": 1.8,
            },
            "past_i64": { "type": "integer", "minimum": 1e30 },
            "under_i64": { "type": "integer", "maximum": -1e30 },
            "wide_open": { "type": "integer", "minimum": -1e30, "maximum": 1e30 },
        }));
        let set = FieldSet::from_properties(&p, &Default::default(), &[]);
        for key in [
            "inverted",
            "inverted_after_rounding",
            "past_i64",
            "under_i64",
        ] {
            assert!(set.by_key(key).unwrap().bounds.unsatisfiable, "{key}");
        }
        // The other direction of the same overflow bounds nothing: every
        // integer is above -1e30 and below 1e30, so the field is free.
        let open = &set.by_key("wide_open").unwrap().bounds;
        assert!(!open.unsatisfiable);
        assert_eq!((open.minimum, open.maximum), (None, None));
    }

    /// An array's element is held to what the dog published about an
    /// element, which is a different schema from the one it published about
    /// the array. The three spellings of an item schema are the three a
    /// `schemars` answer emits for `Vec<Slug>`, `Vec<Option<Slug>>` and an
    /// inline `Vec` of a plain string.
    #[test]
    fn an_arrays_item_bounds_are_read_inline_through_a_ref_and_through_an_anyof() {
        let defs = props(json!({
            "Slug": { "type": "string", "pattern": "^[a-z-]+$", "minLength": 3 },
            "Count": { "type": "integer", "minimum": 1, "maximum": 64 },
        }));
        let p = props(json!({
            "inline": { "type": "array", "items": { "type": "string", "minLength": 2 } },
            "one_hop": { "type": "array", "items": { "$ref": "#/$defs/Slug" } },
            "two_hops": {
                "type": "array",
                "items": { "anyOf": [{ "$ref": "#/$defs/Slug" }, { "type": "null" }] },
            },
            "numbers": { "type": "array", "items": { "$ref": "#/$defs/Count" } },
            "plain": { "type": "array", "items": { "type": "string" } },
            "scalar": { "type": "string", "minLength": 3 },
        }));
        let set = FieldSet::from_properties(&p, &defs, &[]);
        let items = |key: &str| {
            set.by_key(key)
                .expect(key)
                .bounds
                .items
                .as_deref()
                .unwrap_or_else(|| panic!("{key} declares an item schema"))
                .clone()
        };
        assert_eq!(items("inline").min_length, Some(2));
        assert_eq!(items("one_hop").pattern.as_deref(), Some("^[a-z-]+$"));
        assert_eq!(
            items("two_hops").min_length,
            Some(3),
            "the anyOf arm must be resolved before its bounds are read"
        );
        assert_eq!(
            (items("numbers").minimum, items("numbers").maximum),
            (Some(1), Some(64))
        );
        // The negative control: an array of plain strings publishes nothing
        // an element is held to, so a list editor over one refuses nothing.
        assert_eq!(items("plain"), Bounds::default());
        assert_eq!(set.by_key("scalar").unwrap().bounds.items, None);
    }

    /// A `Vec` whose element type is the `Vec` itself, which is what
    /// `schemars` emits for a recursive one. The hop back to a schema
    /// already on the path publishes no item bounds, and reading the field
    /// ends there rather than on the stack.
    #[test]
    fn an_item_schema_that_names_its_own_array_ends_the_walk() {
        let p = props(json!({
            "tree": { "type": "array", "items": { "$ref": "#/$defs/Tree" } },
        }));
        let defs = props(json!({
            "Tree": { "type": "array", "items": { "$ref": "#/$defs/Tree" }, "enum": ["a"] },
        }));
        let set = FieldSet::from_properties(&p, &defs, &[]);
        let bounds = &set.by_key("tree").unwrap().bounds;
        let item = bounds.items.as_deref().expect("the array declares items");
        assert_eq!(
            item.choices,
            vec![json!("a")],
            "the def's own bounds are read"
        );
        assert_eq!(
            item.items.as_deref(),
            None,
            "the hop back to a schema already on the path says nothing"
        );
    }

    /// The same loop, spelled through a `$defs` entry that is only an alias
    /// for another one, which is how a schema author names a type twice.
    ///
    /// The guard has to hold each schema as written beside the one it
    /// resolves to. `B` resolves to `A`, so a path that holds only resolved
    /// values never meets `B` again: the walk saw `B`, resolved it to `A`,
    /// pushed `A`, and found `B` once more for as long as it had stack.
    /// This is a third party's schema, so the crash is an operator's
    /// `shep lookout` opening a dog pane.
    #[test]
    fn an_item_schema_that_is_only_an_alias_for_another_ends_the_walk() {
        let p = props(json!({ "tree": { "$ref": "#/$defs/A" } }));
        let defs = props(json!({
            "A": { "type": "array", "items": { "$ref": "#/$defs/B" }, "minLength": 1 },
            "B": { "$ref": "#/$defs/A" },
        }));
        let set = FieldSet::from_properties(&p, &defs, &[]);
        let bounds = &set.by_key("tree").unwrap().bounds;
        assert_eq!(bounds.min_length, Some(1), "the property's own def");
        let item = bounds.items.as_deref().expect("A declares an item schema");
        assert_eq!(
            item.min_length,
            Some(1),
            "the alias resolves to A, whose bounds are read"
        );
        assert_eq!(
            item.items.as_deref(),
            None,
            "the hop back to A says nothing, and the walk stops one level deeper"
        );
    }

    /// The same loop behind two `anyOf: [T, {type: null}]` arms, which is
    /// how a dog spells an optional field, and the shape the finding named.
    #[test]
    fn an_item_schema_behind_two_nullable_aliases_ends_the_walk() {
        let p = props(json!({ "tree": { "$ref": "#/$defs/A" } }));
        let defs = props(json!({
            "A": { "type": "array", "items": { "$ref": "#/$defs/B" }, "minLength": 1 },
            "B": { "anyOf": [{ "$ref": "#/$defs/C" }, { "type": "null" }] },
            "C": { "anyOf": [{ "$ref": "#/$defs/A" }, { "type": "null" }] },
        }));
        let set = FieldSet::from_properties(&p, &defs, &[]);
        let bounds = &set.by_key("tree").unwrap().bounds;
        let item = bounds.items.as_deref().expect("A declares an item schema");
        assert_eq!(item.min_length, Some(1));
        assert_eq!(item.items.as_deref(), None, "the hop back to A ends it");
    }

    /// An `enum` on an item is the one closed set a list editor can meet: a
    /// scalar's is a cycle widget, and an element is typed.
    #[test]
    fn an_items_enum_is_kept_as_the_values_the_schema_wrote() {
        let p = props(json!({
            "tags": { "type": "array", "items": { "enum": ["ask", "auto"] } },
            "levels": { "type": "array", "items": { "enum": [1, 2, 3] } },
            "open": { "type": "array", "items": { "type": "string" } },
        }));
        let set = FieldSet::from_properties(&p, &Default::default(), &[]);
        let items = |key: &str| {
            set.by_key(key)
                .unwrap()
                .bounds
                .items
                .as_deref()
                .expect("items")
                .clone()
        };
        assert_eq!(items("tags").choices, vec![json!("ask"), json!("auto")]);
        assert_eq!(items("levels").choices, vec![json!(1), json!(2), json!(3)]);
        assert!(items("open").choices.is_empty(), "no enum, no choices");
    }

    /// The bounds the real Flockfile schema publishes, so a change to it
    /// that moved one lands here rather than on an operator.
    ///
    /// `max_memory` and `kill_timeout` carry their grammars through
    /// `$defs`, and `max_restarts` a floor of its own. The two string
    /// grammars go through their own `FromStr` in the gate rather than
    /// through these patterns, so this test is about the reader reaching
    /// them at all.
    #[test]
    fn the_real_schema_publishes_the_bounds_the_gate_reads() {
        let set = real_field_set();
        assert_eq!(
            set.by_key("max_memory").unwrap().bounds.pattern.as_deref(),
            Some(r"^\d+(G|M|K)?$")
        );
        assert_eq!(
            set.by_key("kill_timeout")
                .unwrap()
                .bounds
                .pattern
                .as_deref(),
            Some(r"^\d+(ms|h|m|s)?$")
        );
        assert_eq!(set.by_key("max_restarts").unwrap().bounds.minimum, Some(0));
        assert_eq!(
            set.by_key("cwd").unwrap().bounds,
            Bounds::default(),
            "a plain path field is held to nothing"
        );
    }
}
