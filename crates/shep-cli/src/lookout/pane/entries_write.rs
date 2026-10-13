//! The dog pane's write for a whole-value change to an array or map of
//! tables, which is what removing an entry files.
//!
//! The change is applied to the tables the section already holds, never by
//! rendering the new value: the tables that stay keep the comments and key
//! order the operator wrote, and the ones that go leave with theirs. An entry
//! the section lacks is not created, so a value with more tables than the
//! section has is refused.

use serde_json::{Map, Value};

use super::super::field::Entries;
use super::PathStep;
use super::section_write::{Landed, remove_at, set_at, toml_cell};

/// One leaf of an entry that differs, as the path inside the section to it
/// and what it becomes, or [`None`] for removed.
type Change = (Vec<PathStep>, Option<Value>);

/// Applies `new` as the whole value of the entries field at `path` in `doc`.
///
/// [`Landed::Already`] when the section already holds `new`. [`Landed::Refused`]
/// when `new` is not the shape `kind` holds, when it has an entry the section
/// lacks, and when a changed leaf has no TOML type.
pub(super) fn set_entries(
    doc: &mut toml_edit::DocumentMut,
    kind: Entries,
    path: &[String],
    new: &Value,
) -> Landed {
    let Ok(held) = doc.to_string().parse::<toml::Table>() else {
        return Landed::Refused;
    };
    let held = serde_json::to_value(held).unwrap_or(Value::Null);
    let held = path.iter().try_fold(Some(&held), |at, key| match at {
        Some(Value::Object(map)) => Ok(map.get(key)),
        Some(_) => Err(()),
        None => Ok(None),
    });
    let Ok(held) = held else {
        return Landed::Refused;
    };
    let base: Vec<PathStep> = path.iter().cloned().map(PathStep::Key).collect();
    let plan = match (kind, new) {
        (Entries::Array, Value::Array(new)) => match held {
            Some(Value::Array(held)) => plan_array(&base, held, new),
            None => plan_array(&base, &[], new),
            Some(_) => None,
        },
        (Entries::Map, Value::Object(new)) => match held {
            Some(Value::Object(held)) => plan_map(&base, held, new),
            None => plan_map(&base, &Map::new(), new),
            Some(_) => None,
        },
        _ => None,
    };
    let Some(plan) = plan else {
        return Landed::Refused;
    };
    let mut landed = Landed::Already;
    for (at, value) in &plan.changes {
        let done = match value {
            Some(value) => match toml_cell(value) {
                Some(cell) => set_at(doc.as_table_mut(), at, base.len() + 1, cell, false),
                None => Landed::Refused,
            },
            None => remove_at(doc.as_table_mut(), at, base.len() + 1),
        };
        match done {
            Landed::Changed => landed = Landed::Changed,
            Landed::Already => {}
            Landed::Refused => return Landed::Refused,
        }
    }
    if !plan.removed.is_empty() && drop_entries(doc, path, &plan.removed) {
        landed = Landed::Changed;
    }
    landed
}

/// What turns the held entries into the new ones.
struct Plan {
    /// The leaves that differ in the entries that stay.
    changes: Vec<Change>,
    /// The entries that go.
    removed: Vec<PathStep>,
}

/// The entries of an array of tables, matched to the new ones.
///
/// Content first: a held table identical to a new one is that one. What is
/// left is matched by position, in order, and the new table's leaves that
/// differ become [`Plan::changes`]. [`None`] when there are more new tables
/// than held ones, or a held table is not a table.
fn plan_array(base: &[PathStep], held: &[Value], new: &[Value]) -> Option<Plan> {
    let matched = align(held, new)?;
    let mut changes = Vec::new();
    let mut removed = Vec::new();
    for (index, (table, pair)) in held.iter().zip(&matched).enumerate() {
        match pair {
            Some(new) => {
                let mut at = base.to_vec();
                at.push(PathStep::Index(index));
                diff(table.as_object()?, new.as_object()?, &mut at, &mut changes);
            }
            None => removed.push(PathStep::Index(index)),
        }
    }
    Some(Plan { changes, removed })
}

/// The entries of a map of tables, matched to the new ones by key.
fn plan_map(
    base: &[PathStep],
    held: &Map<String, Value>,
    new: &Map<String, Value>,
) -> Option<Plan> {
    let mut changes = Vec::new();
    let mut removed = Vec::new();
    for (key, table) in held {
        match new.get(key) {
            Some(new) => {
                let mut at = base.to_vec();
                at.push(PathStep::Key(key.clone()));
                diff(table.as_object()?, new.as_object()?, &mut at, &mut changes);
            }
            None => removed.push(PathStep::Key(key.clone())),
        }
    }
    new.keys()
        .all(|key| held.contains_key(key))
        .then_some(Plan { changes, removed })
}

/// For each of `held`, the new table it becomes, or [`None`] for one that
/// goes. The new tables keep their order, and each is matched to a held one
/// that comes later than the one before it.
///
/// The match with the most keys in common wins, and a table identical to a
/// new one outweighs any partial match. Where two held tables are equally
/// good, the earlier stays and the later goes. [`None`] overall when `new` is
/// longer than `held`.
fn align<'a>(held: &[Value], new: &'a [Value]) -> Option<Vec<Option<&'a Value>>> {
    if new.len() > held.len() {
        return None;
    }
    // `best[i][j]`: the most the first `j` new tables score against the first
    // `i` held ones, or [`None`] when they cannot all be placed.
    let mut best = vec![vec![None::<usize>; new.len() + 1]; held.len() + 1];
    for row in &mut best {
        row[0] = Some(0);
    }
    for i in 1..=held.len() {
        for j in 1..=new.len().min(i) {
            let skip = best[i - 1][j];
            let take = best[i - 1][j - 1].map(|score| score + likeness(&held[i - 1], &new[j - 1]));
            best[i][j] = skip.max(take);
        }
    }
    let mut matched = vec![None; held.len()];
    let (mut i, mut j) = (held.len(), new.len());
    while j > 0 {
        if i > j && best[i][j] == best[i - 1][j] {
            i -= 1;
            continue;
        }
        matched[i - 1] = new.get(j - 1);
        i -= 1;
        j -= 1;
    }
    Some(matched)
}

/// How alike two tables are: each key they share with the same value counts
/// twice, and identical tables count once more.
fn likeness(held: &Value, new: &Value) -> usize {
    let (Some(held), Some(new)) = (held.as_object(), new.as_object()) else {
        return 0;
    };
    let shared = new
        .iter()
        .filter(|(key, value)| held.get(*key) == Some(*value))
        .count();
    2 * shared + usize::from(held == new)
}

/// Pushes onto `out` every leaf under `at` that `new` has different from
/// `held`: a key `new` lacks is removed, and a table is entered rather than
/// replaced.
fn diff(
    held: &Map<String, Value>,
    new: &Map<String, Value>,
    at: &mut Vec<PathStep>,
    out: &mut Vec<Change>,
) {
    let none = Map::new();
    for (key, value) in new {
        let before = held.get(key);
        if before == Some(value) {
            continue;
        }
        at.push(PathStep::Key(key.clone()));
        match value {
            Value::Object(inner) => {
                diff(
                    before.and_then(Value::as_object).unwrap_or(&none),
                    inner,
                    at,
                    out,
                );
            }
            _ => out.push((at.clone(), Some(value.clone()))),
        }
        at.pop();
    }
    for key in held.keys().filter(|key| !new.contains_key(*key)) {
        let mut gone = at.clone();
        gone.push(PathStep::Key(key.clone()));
        out.push((gone, None));
    }
}

/// Deletes the entries `gone` names from the entries field at `path`, in
/// place, and reports whether there was anything to delete.
///
/// An array of tables left with none is removed from its parent, so the key
/// is written as no `[[x]]` at all.
fn drop_entries(doc: &mut toml_edit::DocumentMut, path: &[String], gone: &[PathStep]) -> bool {
    let Some((last, parents)) = path.split_last() else {
        return false;
    };
    let Some(parent) = table_at(doc.as_table_mut(), parents) else {
        return false;
    };
    let Some(item) = parent.get_mut(last) else {
        return false;
    };
    let mut changed = false;
    for step in gone.iter().rev() {
        changed |= match (&mut *item, step) {
            (toml_edit::Item::ArrayOfTables(tables), PathStep::Index(at)) if *at < tables.len() => {
                tables.remove(*at);
                true
            }
            (toml_edit::Item::Value(toml_edit::Value::Array(array)), PathStep::Index(at))
                if *at < array.len() =>
            {
                let gone = array.remove(*at);
                // The first element carries the space after `[`, so it moves
                // to the one that is first now.
                if *at == 0
                    && let (Some(prefix), Some(first)) =
                        (gone.decor().prefix().cloned(), array.get_mut(0))
                {
                    first.decor_mut().set_prefix(prefix);
                }
                true
            }
            (item, PathStep::Key(key)) => item
                .as_table_like_mut()
                .is_some_and(|table| table.remove(key).is_some()),
            _ => false,
        };
    }
    if matches!(item, toml_edit::Item::ArrayOfTables(tables) if tables.is_empty()) {
        parent.remove(last);
    }
    changed
}

/// The table `keys` name below `table`.
fn table_at<'a>(
    table: &'a mut dyn toml_edit::TableLike,
    keys: &[String],
) -> Option<&'a mut dyn toml_edit::TableLike> {
    match keys.split_first() {
        None => Some(table),
        Some((key, rest)) => table_at(table.get_mut(key)?.as_table_like_mut()?, rest),
    }
}
