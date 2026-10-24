//! The `dogs.toml` half of `shep set`: one key written into the section text
//! the shepherd answered, in the form that text already spells it in.
//!
//! [`shep_core::protocol::Request::SetDogConfig`] replaces a whole section, so
//! the only way a comment the operator wrote inside one survives the write is
//! to change the text that was read rather than render a map back. Every
//! helper here works on a `toml_edit` document for that reason.

use serde_json::{Map, Value as Json};
use toml_edit::{Decor, DocumentMut, InlineTable, Item, Table, Value as TomlValue};

use super::WriteRefusal;
use super::value::{inline_table, null_leaf, toml_value};

/// Why a `toml_edit` write stopped, naming the dotted path, under the table it
/// was writing, that it stopped at.
///
/// The ways a write can stop are separate sentences an operator needs: a parent
/// that is not a table, a table the value would erase, an array of tables, and
/// a value with no TOML cell to write. Reporting one of them as another is how
/// an operator is told a table "is not a table".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Held {
    /// The path holds something this write would have to turn into a table to
    /// reach the key it was asked for.
    Through(String),
    /// The path holds a table, and the value would erase it.
    Table(String),
    /// The path holds an array of tables, spelled `[[path]]`.
    ArrayOfTables(String),
    /// The value, or something under it, has no TOML cell to write.
    Untyped(String),
}

impl Held {
    /// The path this refusal stopped at, `""` for the table itself.
    fn path(&self) -> &str {
        match self {
            Self::Through(path)
            | Self::Table(path)
            | Self::ArrayOfTables(path)
            | Self::Untyped(path) => path,
        }
    }

    /// The sentence the operator reads, naming the whole address the write was
    /// aimed at rather than the fragment this module walked.
    pub(super) fn refusal(self, name: &str, keys: &[String]) -> WriteRefusal {
        let at = match self.path() {
            "" => name.to_string(),
            path => format!("{name}.{path}"),
        };
        match self {
            Self::Through(_) => WriteRefusal::Refused(format!(
                "`{at}` holds a value that is not a table, and `shep set` cannot write `{}` \
                 through it",
                keys.join(".")
            )),
            Self::Table(_) => WriteRefusal::Refused(format!(
                "`{at}` holds a table, and `shep set` does not replace a table with a value that \
                 is not one"
            )),
            Self::ArrayOfTables(_) => WriteRefusal::Refused(format!(
                "`{at}` is an array of tables, and `shep set` writes one table, not an entry of \
                 one"
            )),
            Self::Untyped(_) => WriteRefusal::Invalid(format!(
                "nothing was written: `{at}` has no TOML to write, and a `dogs.toml` section is \
                 written as TOML"
            )),
        }
    }
}

/// `map` as a whole `dogs.toml` section document.
///
/// The one write that builds a document rather than editing one, and the only
/// thing that loses a comment: an operator replacing a whole section asked for
/// the section they are writing.
pub(super) fn document_of(map: &Map<String, Json>) -> Result<DocumentMut, Held> {
    refuse_nulls(&Json::Object(map.clone()), "")?;
    let mut doc = DocumentMut::new();
    for (key, value) in map {
        if let Some(cell) = toml_value(value) {
            doc.as_table_mut().insert(key.as_str(), Item::Value(cell));
        }
    }
    Ok(doc)
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
/// # Errors
/// [`Held`] naming the dotted path, under `table`, the write stopped at.
/// Writing through a scalar is a change to that value, and the caller reports
/// it rather than making it on the way to the key it was asked to write.
pub(super) fn set_toml(
    table: &mut Table,
    keys: &[String],
    held: &str,
    value: &Json,
) -> Result<(), Held> {
    let Some((head, rest)) = keys.split_first() else {
        return Err(Held::Through(held.to_string()));
    };
    let walked = under(held, head);
    // `entry` rather than `insert`: `insert` re-formats the key it replaces,
    // and that is where a comment the operator wrote above the key goes.
    let entry = table.entry(head).or_insert(Item::None);
    if rest.is_empty() {
        return write_leaf(entry, &walked, value);
    }
    if entry.is_none() {
        let mut nested = Table::new();
        nested.set_implicit(true);
        *entry = Item::Table(nested);
    }
    match entry {
        // A `[worker]` header and a dotted `worker.timeout = 30` are both a
        // `Table` here. An inline table holds `Value`s and no `Item`s, so it
        // walks through its own door.
        Item::Table(nested) => set_toml(nested, rest, &walked, value),
        Item::Value(TomlValue::InlineTable(inline)) => set_inline(inline, rest, &walked, value),
        Item::ArrayOfTables(_) => Err(Held::ArrayOfTables(walked)),
        // `Item::None` is unreachable here: the branch above fills a parent the
        // document lacks. Naming it keeps the match total rather than leaving a
        // write that reaches it to a compiler's opinion.
        Item::Value(_) | Item::None => Err(Held::Through(walked)),
    }
}

/// One key at the end of the walk: the form the key already has is the form it
/// keeps. A `[worker]` header stays a header with its comment, a `worker = { .. }`
/// inline table stays inline, and a key the document does not have yet is
/// written as a header, because that is the form a `dogs.toml` section spells a
/// table in.
fn write_leaf(entry: &mut Item, walked: &str, value: &Json) -> Result<(), Held> {
    // An array of tables is its own shape: replacing `[[sinks]]` with one table
    // drops every entry but the one being written.
    if matches!(entry, Item::ArrayOfTables(_)) {
        return Err(Held::ArrayOfTables(walked.to_string()));
    }
    if entry.is_table_like() && !value.is_object() {
        return Err(Held::Table(walked.to_string()));
    }
    refuse_nulls(value, walked)?;
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
        other => Item::Value(toml_value(other).ok_or_else(|| Held::Untyped(walked.to_string()))?),
    };
    Ok(())
}

/// [`set_toml`] walking an inline table, which holds `Value`s rather than
/// `Item`s and can hold no `[header]`: a table written under one is inline, in
/// the form the parent is already in.
fn set_inline(
    inline: &mut InlineTable,
    keys: &[String],
    held: &str,
    value: &Json,
) -> Result<(), Held> {
    let Some((head, rest)) = keys.split_first() else {
        return Err(Held::Through(held.to_string()));
    };
    let walked = under(held, head);
    if rest.is_empty() {
        if inline.get(head).is_some_and(TomlValue::is_inline_table) && !value.is_object() {
            return Err(Held::Table(walked));
        }
        refuse_nulls(value, &walked)?;
        let cell = toml_value(value).ok_or_else(|| Held::Untyped(walked.clone()))?;
        inline.insert(head, cell);
        return Ok(());
    }
    if let Some(TomlValue::InlineTable(nested)) = inline.get_mut(head) {
        return set_inline(nested, rest, &walked, value);
    }
    if inline.get(head).is_some() {
        return Err(Held::Through(walked));
    }
    let mut nested = InlineTable::new();
    set_inline(&mut nested, rest, &walked, value)?;
    inline.insert(head, TomlValue::InlineTable(nested));
    Ok(())
}

/// `head` under the path this walk has reached.
fn under(held: &str, head: &str) -> String {
    if held.is_empty() {
        head.to_string()
    } else {
        format!("{held}.{head}")
    }
}

/// `Err` when `value` puts a null anywhere under the key this write is at.
///
/// The path is named from the table this walks rather than from the value, so
/// the refusal reads as the address the operator typed: a null inside
/// `{ keep = null }` written at `limits` sits at `limits.keep`.
fn refuse_nulls(value: &Json, walked: &str) -> Result<(), Held> {
    let Json::Object(map) = value else {
        return Ok(());
    };
    let Some(under_value) = null_leaf(map) else {
        return Ok(());
    };
    Err(Held::Untyped(if walked.is_empty() {
        under_value
    } else {
        format!("{walked}.{under_value}")
    }))
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
