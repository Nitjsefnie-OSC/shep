//! What a copy would write, and every key it changes on the target.
//!
//! The plan is built entirely from the two configs this command already read,
//! so the operator sees the whole change list before a byte is written, and
//! the write is one request the shepherd already answers.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use shep_core::config::DogTable;

use super::CopyRefusal;

/// The request a plan turns into.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Write {
    /// One `AppConfig` field, through `Request::SetSheepField`.
    Field {
        /// The field name.
        name: String,
        /// The value the source holds for it.
        value: Value,
    },
    /// One dog's whole table, through `Request::SetSheepDogSettings`.
    ///
    /// The whole table and not the copied slice: that request replaces a
    /// dog's table, so the merged result is what carries the keys the copy
    /// left alone.
    DogTable {
        /// The dog's name.
        dog: String,
        /// The table the target holds afterwards.
        table: DogTable,
    },
}

/// What one key ends up as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    /// The target had no such key.
    Added,
    /// The target had one, holding something else.
    Changed,
    /// The target had one and the copy leaves it out.
    Removed,
}

impl ChangeKind {
    /// The word the table column and the JSON field carry.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Changed => "changed",
            Self::Removed => "removed",
        }
    }
}

/// One key the copy touches.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Change {
    /// The key, dotted, under the path the copy names.
    pub(crate) key: String,
    /// What happens to it.
    pub(crate) kind: ChangeKind,
    /// What the target held, `None` for an added key.
    pub(crate) old: Option<Value>,
    /// What the target holds afterwards, `None` for a removed key.
    pub(crate) new: Option<Value>,
}

/// The whole copy: what changes, and what is sent for it.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Plan {
    /// One entry per key the copy touches, in dotted-key order.
    pub(crate) changes: Vec<Change>,
    /// The write, `None` when nothing differs and there is nothing to send.
    pub(crate) write: Option<Write>,
}

/// The plan for one `AppConfig` field.
///
/// `source` and `target` are the two sheep's configs as JSON objects.
#[must_use]
pub(crate) fn plan_field(field: &str, source: &Value, target: &Value) -> Plan {
    let Some(value) = source.get(field) else {
        return Plan::default();
    };
    let current = target.get(field);
    if current == Some(value) {
        return Plan::default();
    }
    Plan {
        changes: vec![Change {
            key: field.to_string(),
            kind: ChangeKind::Changed,
            old: current.cloned(),
            new: Some(value.clone()),
        }],
        write: Some(Write::Field {
            name: field.to_string(),
            value: value.clone(),
        }),
    }
}

/// The plan for one dog's table, or one key inside it.
///
/// `keys` is the path under the dog's table, empty for the whole table.
/// `except` names keys under that same path.
///
/// # Errors
/// [`CopyRefusal::MissingSource`] when the source sheep has no table for the
/// dog, or no key at `keys`.
pub(crate) fn plan_dog(
    source_sheep: &str,
    dog: &str,
    keys: &[String],
    except: &[Vec<String>],
    replace: bool,
    source: &Value,
    target: &Value,
) -> Result<Plan, CopyRefusal> {
    let source_table = dogs_of(source).and_then(|dogs| dogs.get(dog));
    let source_here = if keys.is_empty() {
        source_table
    } else {
        source_table.and_then(|table| walk(table, keys))
    };
    let Some(source_here) = source_here else {
        return Err(if source_table.is_none() {
            CopyRefusal::MissingSource(format!(
                "no `{}.dogs.{dog}`: that sheep has no table for the dog `{dog}`",
                source_sheep
            ))
        } else {
            CopyRefusal::MissingSource(format!(
                "no `{}` under `{}.dogs.{dog}`",
                keys.join("."),
                source_sheep
            ))
        });
    };

    let before = Value::Object(
        dogs_of(target)
            .and_then(|dogs| dogs.get(dog))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    );
    let mut after = before.clone();
    if keys.is_empty() {
        let merged = merge(Some(&after), source_here, replace, except);
        after = merged;
    } else {
        let merged = merge(walk(&after, keys), source_here, replace, except);
        set_path(&mut after, keys, merged);
    }

    let before_here: Option<&Value> = if keys.is_empty() {
        Some(&before)
    } else {
        walk(&before, keys)
    };
    let after_here: Option<&Value> = if keys.is_empty() {
        Some(&after)
    } else {
        walk(&after, keys)
    };

    let mut changes = Vec::new();
    let root = if after_here.is_some_and(Value::is_object) {
        String::new()
    } else {
        keys.last()
            .map(String::as_str)
            .unwrap_or_default()
            .to_string()
    };
    collect_changes(before_here, after_here, &root, &mut changes);

    let write = (!changes.is_empty()).then(|| {
        after.as_object().cloned().map(|table| Write::DogTable {
            dog: dog.to_string(),
            table: DogTable::from(table),
        })
    });

    Ok(Plan {
        changes,
        write: write.flatten(),
    })
}

/// The `--except` arguments, as key paths under the copied path.
///
/// # Errors
/// [`CopyRefusal::Usage`] for a spec with an empty part, which cannot name a
/// key.
pub(crate) fn except_paths(raw: &[String]) -> Result<Vec<Vec<String>>, CopyRefusal> {
    raw.iter()
        .map(|spec| {
            let parts: Vec<&str> = spec.split('.').collect();
            if parts.iter().any(|part| part.is_empty()) {
                return Err(CopyRefusal::Usage(format!(
                    "`--except {spec}` has an empty part: a key is named as `git.checkout`, not \
                     as a pattern"
                )));
            }
            Ok(parts.into_iter().map(str::to_string).collect())
        })
        .collect()
}

/// The `dogs` map of one config, as JSON.
fn dogs_of(config: &Value) -> Option<&Map<String, Value>> {
    config.get("dogs").and_then(Value::as_object)
}

/// The value at `keys` under `root`, `None` when any step is missing or is
/// not a table.
fn walk<'a>(root: &'a Value, keys: &[String]) -> Option<&'a Value> {
    let mut node = root;
    for key in keys {
        node = node.as_object()?.get(key)?;
    }
    Some(node)
}

/// Writes `value` at `keys` under `root`, creating the tables on the way.
fn set_path(root: &mut Value, keys: &[String], value: Value) {
    let Some((head, rest)) = keys.split_first() else {
        *root = value;
        return;
    };
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    // The line above is what makes this the only reachable arm.
    let Some(map) = root.as_object_mut() else {
        return;
    };
    let entry = map
        .entry(head.clone())
        .or_insert_with(|| Value::Object(Map::new()));
    set_path(entry, rest, value);
}

/// Removes `keys` from `root`, if it is there.
fn remove_path(root: &mut Value, keys: &[String]) {
    let Some((head, rest)) = keys.split_first() else {
        return;
    };
    let Some(map) = root.as_object_mut() else {
        return;
    };
    if rest.is_empty() {
        map.remove(head);
        return;
    }
    if let Some(child) = map.get_mut(head) {
        remove_path(child, rest);
    }
}

/// The table the target ends up with at the copied path.
///
/// Merged by default: a table keeps the target's own keys, and a table on
/// both sides merges key by key so a `--except` deep inside one is honoured.
/// A scalar or an array in the source replaces the target's entry whole.
/// `replace` starts from the source instead of the target.
///
/// `except` are paths under this node. One that names this node's own key is
/// handled by the caller that recursed here.
fn merge(target: Option<&Value>, source: &Value, replace: bool, except: &[Vec<String>]) -> Value {
    if replace {
        let mut out = source.clone();
        for path in except {
            remove_path(&mut out, path);
            if let Some(value) = target.and_then(|held| walk(held, path)) {
                set_path(&mut out, path, value.clone());
            }
        }
        return out;
    }
    let (Some(Value::Object(target_map)), Value::Object(source_map)) = (target, source) else {
        return source.clone();
    };
    let mut out = target_map.clone();
    for (key, value) in source_map {
        let nested = nested_excepts(except, key);
        if nested.iter().any(Vec::is_empty) {
            continue;
        }
        out.insert(
            key.clone(),
            merge(target_map.get(key), value, false, &nested),
        );
    }
    Value::Object(out)
}

/// The `--except` paths that live under `key`, with `key` off the front.
fn nested_excepts(except: &[Vec<String>], key: &str) -> Vec<Vec<String>> {
    except
        .iter()
        .filter(|path| path.first().map(String::as_str) == Some(key))
        .map(|path| path[1..].to_vec())
        .collect()
}

/// Every key that differs between `before` and `after`, as dotted paths under
/// `key`. A key holding the same value on both sides is no change.
fn collect_changes(
    before: Option<&Value>,
    after: Option<&Value>,
    key: &str,
    out: &mut Vec<Change>,
) {
    match (before, after) {
        (Some(Value::Object(before)), Some(Value::Object(after))) => {
            let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
            for name in keys {
                let child = child_key(key, name);
                collect_changes(before.get(name), after.get(name), &child, out);
            }
        }
        (Some(Value::Object(before)), None) => {
            for name in before.keys() {
                let child = child_key(key, name);
                collect_changes(Some(&before[name]), None, &child, out);
            }
        }
        (None, Some(Value::Object(after))) => {
            for name in after.keys() {
                let child = child_key(key, name);
                collect_changes(None, Some(&after[name]), &child, out);
            }
        }
        (Some(before), Some(after)) => {
            if before != after {
                out.push(Change {
                    key: key.to_string(),
                    kind: ChangeKind::Changed,
                    old: Some(before.clone()),
                    new: Some(after.clone()),
                });
            }
        }
        (Some(before), None) => out.push(Change {
            key: key.to_string(),
            kind: ChangeKind::Removed,
            old: Some(before.clone()),
            new: None,
        }),
        (None, Some(after)) => out.push(Change {
            key: key.to_string(),
            kind: ChangeKind::Added,
            old: None,
            new: Some(after.clone()),
        }),
        (None, None) => {}
    }
}

/// A child key under `key`, which is bare at the copied path itself.
fn child_key(key: &str, name: &str) -> String {
    if key.is_empty() {
        name.to_string()
    } else {
        format!("{key}.{name}")
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn table(pairs: &[(&str, Value)]) -> Value {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert((*key).to_string(), value.clone());
        }
        json!({ "dogs": { "kelpie": map } })
    }

    /// The keys a plan reports are the keys under the path the operator named.
    #[test]
    fn a_deep_path_reports_its_keys_under_itself() {
        let source = json!({ "dogs": { "kelpie": { "rulings": { "panel": "panel" } } } });
        let target = json!({ "dogs": { "kelpie": { "rulings": { "panel": "issue" } } } });
        let plan = plan_dog(
            "shep",
            "kelpie",
            &["rulings".to_string()],
            &[],
            false,
            &source,
            &target,
        )
        .unwrap();
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].key, "panel");
    }

    #[test]
    fn a_nested_table_merges_and_an_array_overwrites_whole() {
        let source =
            json!({ "dogs": { "kelpie": { "limits": { "a": 1, "b": 2 }, "tags": ["x", "y"] } } });
        let target =
            json!({ "dogs": { "kelpie": { "limits": { "b": 9, "c": 3 }, "tags": ["z"] } } });
        let plan = plan_dog("shep", "kelpie", &[], &[], false, &source, &target).unwrap();
        let keys: Vec<&str> = plan.changes.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["limits.a", "limits.b", "tags"]);
        let Write::DogTable { table, .. } = plan.write.unwrap() else {
            panic!("a dog copy writes a dog table");
        };
        assert_eq!(
            table.as_map()["limits"],
            json!({ "a": 1, "b": 2, "c": 3 }),
            "a table on both sides merges key by key, so `c` survives"
        );
        assert_eq!(
            table.as_map()["tags"],
            json!(["x", "y"]),
            "an array is one value, not a table"
        );
    }

    #[test]
    fn a_table_merges_key_by_key_and_keeps_what_only_the_target_has() {
        let source = table(&[("git", json!({ "checkout": "./src", "remote": "origin" }))]);
        let target = table(&[("git", json!({ "checkout": "./other", "own": "keep" }))]);
        let plan = plan_dog("shep", "kelpie", &[], &[], false, &source, &target).unwrap();
        let keys: Vec<&str> = plan.changes.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["git.checkout", "git.remote"]);
        let Write::DogTable { table, .. } = plan.write.unwrap() else {
            panic!("a dog copy writes a dog table");
        };
        assert_eq!(
            table.as_map()["git"],
            json!({ "checkout": "./src", "own": "keep", "remote": "origin" })
        );
    }

    #[test]
    fn an_excepted_key_is_pruned_from_the_source_and_restored_under_replace() {
        let source = table(&[("git", json!({ "checkout": "./src", "remote": "origin" }))]);
        let target = table(&[("git", json!({ "checkout": "./other" }))]);
        let except = [vec!["git".to_string(), "checkout".to_string()]];

        let merged = plan_dog("shep", "kelpie", &[], &except, false, &source, &target).unwrap();
        let Write::DogTable { table, .. } = merged.write.unwrap() else {
            panic!("a dog copy writes a dog table");
        };
        assert_eq!(
            table.as_map()["git"],
            json!({ "checkout": "./other", "remote": "origin" })
        );

        let replaced = plan_dog("shep", "kelpie", &[], &except, true, &source, &target).unwrap();
        let Write::DogTable { table, .. } = replaced.write.unwrap() else {
            panic!("a dog copy writes a dog table");
        };
        assert_eq!(
            table.as_map()["git"],
            json!({ "checkout": "./other", "remote": "origin" })
        );
    }

    #[test]
    fn a_missing_except_part_is_a_usage_error() {
        let err = except_paths(&["git..remote".to_string()]).unwrap_err();
        assert!(err.message().contains("--except git..remote"), "{err}");
    }

    #[test]
    fn a_field_that_already_matches_needs_no_write() {
        let plan = plan_field(
            "restart_delay",
            &json!({ "restart_delay": "3s" }),
            &json!({ "restart_delay": "3s" }),
        );
        assert!(plan.changes.is_empty());
        assert_eq!(plan.write, None);
    }
}
