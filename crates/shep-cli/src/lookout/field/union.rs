//! A setting that is one of several object shapes, told apart by a tag.
//!
//! serde's internally tagged enum (`#[serde(tag = "kind")]`) comes out of
//! schemars as a `oneOf` with one object per variant, each carrying a
//! `const` on the shared tag property. [`union_of`] reads that shape.

use serde_json::{Map, Value};

use super::schema::{holds_secret, resolved};
use super::{Field, FieldKind, field_from};

/// The control character the pane joins a union's name, an arm's tag and a
/// field's name with to key the rows under it. A union with a name that
/// holds it would key two rows alike, so [`union_of`] reads it as no union.
pub(crate) const KEY_SEP: char = '\u{1f}';

/// A [`FieldKind::Union`]: the tag property's name and the arms it picks
/// between, in the schema's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Union {
    /// The property every arm carries a `const` on.
    pub tag: String,
    /// One per `oneOf` arm.
    pub arms: Vec<Arm>,
}

/// One shape of a [`Union`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arm {
    /// The `const` this arm holds on the union's tag.
    pub tag: String,
    /// The arm's own `description`, for the blurb panel.
    pub description: Option<String>,
    /// The arm's properties other than the tag. A property with no editor in
    /// this pane is [`FieldKind::Opaque`] and read-only.
    pub fields: Vec<Field>,
    required: Vec<String>,
}

impl Union {
    /// The arm whose tag is `tag`.
    #[must_use]
    pub fn arm(&self, tag: &str) -> Option<&Arm> {
        self.arms.iter().find(|arm| arm.tag == tag)
    }

    /// The tag property, every arm's tag and every arm field's name.
    fn names(&self) -> impl Iterator<Item = &str> {
        core::iter::once(self.tag.as_str()).chain(self.arms.iter().flat_map(|arm| {
            core::iter::once(arm.tag.as_str()).chain(arm.fields.iter().map(|f| f.key.as_str()))
        }))
    }

    /// Every arm's tag, in the schema's order.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.arms.iter().map(|arm| arm.tag.as_str())
    }
}

impl Arm {
    /// Whether the schema lists `key` among this arm's required properties.
    #[must_use]
    pub fn requires(&self, key: &str) -> bool {
        self.required.iter().any(|name| name == key)
    }
}

/// `schema` as a [`Union`], or [`None`] when it is not one: no `oneOf`, an
/// arm that is not an object, no property that every arm requires and holds
/// a string `const` on, two arms with the same tag, or a name holding
/// [`KEY_SEP`]. A union that reaches a
/// secret is not one either: its row would have to mask a value it also
/// names.
pub(super) fn union_of(schema: &Value, defs: &Map<String, Value>) -> Option<Union> {
    let arms: Vec<&Value> = schema
        .get("oneOf")?
        .as_array()?
        .iter()
        .map(|arm| resolved(arm, defs))
        .collect();
    if arms.is_empty() || holds_secret(schema, defs) {
        return None;
    }
    let tag = arms
        .first()?
        .get("properties")?
        .as_object()?
        .keys()
        .find(|name| arms.iter().all(|arm| tag_of(arm, name).is_some()))?
        .clone();
    let mut built: Vec<Arm> = Vec::with_capacity(arms.len());
    for arm in arms {
        let value = tag_of(arm, &tag)?;
        if built.iter().any(|seen| seen.tag == value) {
            return None;
        }
        built.push(Arm {
            tag: value.to_owned(),
            description: arm
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            fields: arm_fields(arm, &tag, defs),
            required: required_of(arm),
        });
    }
    let union = Union { tag, arms: built };
    let plain = union.names().all(|name| !name.contains(KEY_SEP));
    plain.then_some(union)
}

/// The string `const` that `arm` holds on `name`, when `arm` is an object
/// that requires `name`.
fn tag_of<'a>(arm: &'a Value, name: &str) -> Option<&'a str> {
    if arm.get("type").and_then(Value::as_str) != Some("object")
        || !required_of(arm).contains(&name.to_owned())
    {
        return None;
    }
    arm.get("properties")?.get(name)?.get("const")?.as_str()
}

fn required_of(arm: &Value) -> Vec<String> {
    arm.get("required")
        .and_then(Value::as_array)
        .map(|names| {
            names
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The properties of `arm` other than `tag`. Only a scalar, a choice and a
/// list get an editor: a nested union, a map and a table would each need a
/// sub-screen the arm's rows do not have, and a union that held itself would
/// not end.
fn arm_fields(arm: &Value, tag: &str, defs: &Map<String, Value>) -> Vec<Field> {
    let Some(properties) = arm.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    properties
        .iter()
        .filter(|(name, _)| name.as_str() != tag)
        .map(|(name, schema)| {
            let mut field = field_from(name, schema, defs);
            if !matches!(
                field.kind,
                FieldKind::Bool
                    | FieldKind::Integer
                    | FieldKind::Number
                    | FieldKind::Text
                    | FieldKind::Choice(_)
                    | FieldKind::Suggested(_)
                    | FieldKind::List(_)
            ) {
                field.kind = FieldKind::Opaque;
                field.editable = false;
            }
            field
        })
        .collect()
}
