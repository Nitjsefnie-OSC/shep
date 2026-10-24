//! The `dogs.toml` half of `shep set`: one key written into the section text
//! the shepherd answered, in the form that text already spells it in.
//!
//! [`shep_core::protocol::Request::SetDogConfig`] replaces a whole section, so
//! the only way a comment the operator wrote inside one survives the write is
//! to change the text that was read rather than render a map back. Every
//! helper here works on a `toml_edit` document for that reason.

use serde_json::{Map, Value as Json};
use toml_edit::{Decor, DocumentMut, Item, Table, Value as TomlValue};

use super::value::{inline_table, toml_value};

/// `map` as a whole `dogs.toml` section document.
///
/// The one write that builds a document rather than editing one, and the only
/// thing that loses a comment: an operator replacing a whole section asked for
/// the section they are writing.
pub(super) fn document_of(map: &Map<String, Json>) -> DocumentMut {
    let mut doc = DocumentMut::new();
    for (key, value) in map {
        if let Some(cell) = toml_value(value) {
            doc.as_table_mut().insert(key.as_str(), Item::Value(cell));
        }
    }
    doc
}

/// Writes `value` at `keys` in `table`, creating any table the document does
/// not carry yet.
///
/// A parent that is there takes the write where it is: a `[worker]` header, a
/// `worker.timeout = 30` dotted key and a `worker = { timeout = 30 }` inline
/// table each render back in the form they came in. A parent the document
/// lacks is created implicit, so the write adds `[worker.timeout]` and no
/// empty `[worker]` header above it.
///
/// `Err(held)` names the dotted path, under `table`, of the value this write
/// would have to turn into a table, or the leaf it would have to turn into a
/// scalar. Writing through a scalar is a change to that value, and the caller
/// reports it rather than making it on the way to the key it was asked to
/// write.
pub(super) fn set_toml(
    table: &mut Table,
    keys: &[String],
    held: &str,
    value: &Json,
) -> Result<(), String> {
    let Some((head, rest)) = keys.split_first() else {
        return Err(held.to_string());
    };
    let walked = if held.is_empty() {
        head.clone()
    } else {
        format!("{held}.{head}")
    };
    // `entry` rather than `insert`: `insert` re-formats the key it replaces,
    // and that is where a comment the operator wrote above the key goes.
    let entry = table.entry(head).or_insert(Item::None);
    if rest.is_empty() {
        if entry.is_table_like() && !value.is_object() {
            return Err(walked);
        }
        // The form the key already has is the form it keeps: a `[worker]` header
        // stays a header with its comment, a `worker = { .. }` inline table stays
        // inline. A key the document does not have yet is written as a header,
        // because that is the form a `dogs.toml` section spells a table in.
        let decor = match entry {
            Item::Table(table) => Some(table.decor().clone()),
            Item::None => Some(Decor::new("\n", "")),
            Item::Value(_) | Item::ArrayOfTables(_) => None,
        };
        *entry = match value {
            Json::Object(map) => match decor {
                Some(decor) => Item::Table(header_table(map, decor)),
                None => Item::Value(TomlValue::InlineTable(inline_table(map))),
            },
            other => Item::Value(toml_value(other).ok_or(walked)?),
        };
        return Ok(());
    }
    if entry.is_none() {
        let mut nested = Table::new();
        nested.set_implicit(true);
        *entry = Item::Table(nested);
    } else if !entry.is_table_like() {
        return Err(walked);
    }
    let Some(nested) = entry.as_table_mut() else {
        return Err(walked);
    };
    set_toml(nested, rest, &walked, value)
}

/// A `[header]` table holding `map`, keeping the decor `old` carried, which is
/// where the comment above the header sits.
fn header_table(map: &Map<String, Json>, decor: Decor) -> Table {
    let mut nested = Table::new();
    for (key, value) in map {
        if let Some(cell) = toml_value(value) {
            nested.insert(key.as_str(), Item::Value(cell));
        }
    }
    *nested.decor_mut() = decor;
    nested
}

/// `held` as the dotted suffix [`crate::commands::copy::plan::set_path`] names
/// a held value with.
pub(super) fn dotted(held: &str) -> String {
    if held.is_empty() {
        String::new()
    } else {
        format!(".{held}")
    }
}
