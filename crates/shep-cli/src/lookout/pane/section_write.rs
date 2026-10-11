//! The dog pane's write: every filed edit applied to the section's own TOML
//! text, ready for `Request::SetDogConfig`.
//!
//! `toml_edit` rather than a re-render of the pane's parsed values, and that
//! is the whole reason this module exists: the request replaces the table
//! wholesale, so a re-render would delete every comment in it on the
//! operator's own keystroke. `dogs.toml` is the operator's file, and shep
//! never writes it any other way.
//!
//! A row key is dotted (`models.worker.model`) whenever the dog's schema
//! nests a table, and the pane keeps the key-to-path map [`flattened`] built
//! the rows from beside the section text. The path is what this writer walks,
//! never the key: a property literally named `a.b` is one key, and the
//! section spells it `"a.b" = ...`.

use serde_json::Value;

use super::super::edits::{EditKey, Edits};
#[cfg(doc)]
use super::super::field::flattened;
use super::{ConfigPane, PaneEdit};

impl ConfigPane {
    /// The section with every field edit in `edits` applied at its own path,
    /// comments and key order intact, ready for `Request::SetDogConfig`.
    ///
    /// This is what a close actually sends: the request replaces the table,
    /// so a batch of edits to one dog is one write, not one per entry. Each
    /// is applied to the document the previous one produced, walked in
    /// [`Edits::iter`]'s own key order.
    ///
    /// An [`EditKey::Env`] entry has no home in a dog's section, a dog
    /// having no env store, and is skipped rather than becoming a key in it.
    ///
    /// A `null` value removes the leaf and nothing above it, which is how the
    /// pane's empty buffer unsets one, and is what puts the dog back on its
    /// own default. A `null` on a key the section does not carry changes
    /// nothing and is not a failure: the section already says the key is
    /// absent, and refusing such an edit would cost every other edit in the
    /// close its write.
    ///
    /// [`None`] for a sheep pane, for a set holding no field edit, for a
    /// section that does not parse, and for any one entry this writer cannot
    /// land: a value with no TOML type, or a path that lands on something
    /// other than the shape the row edits (a scalar where the schema says a
    /// table, a table where the row edits one key inside it). A partial
    /// section is worse than none, since the request would replace the table
    /// with it.
    #[must_use]
    pub fn edited_section_with(&self, edits: &Edits) -> Option<String> {
        let section = self.section.as_deref()?;
        let mut doc: toml_edit::DocumentMut = section.parse().ok()?;
        let mut changed = false;
        for (key, edit) in edits.iter() {
            let EditKey::Field(name) = key else {
                continue;
            };
            let PaneEdit::Set { value, .. } = edit.edit() else {
                continue;
            };
            let value = value.as_value();
            // A filed key always names a row `flattened` built. The one-key
            // fallback is for a key that somehow does not: the whole name as
            // one literal key, never split on its dots.
            let unnamed = vec![name.clone()];
            let path = self.section_paths.get(name).unwrap_or(&unnamed);
            let landed = match toml_cell(value) {
                Some(cell) => set_at(doc.as_table_mut(), path, cell),
                // `null` has no TOML cell: it is the removal, not a value.
                None if value.is_null() => remove_at(doc.as_table_mut(), path),
                None => Landed::Refused,
            };
            match landed {
                Landed::Changed => changed = true,
                Landed::Already => {}
                Landed::Refused => return None,
            }
        }
        changed.then(|| doc.to_string())
    }
}

/// What one edit left behind it in the document.
///
/// Three states because a batch is judged as a whole: one entry that cannot
/// land loses the whole close, while an entry the section already carries must
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Landed {
    /// The document now says something it did not before.
    Changed,
    /// The document already said this, so it is unchanged: a `null` on a key
    /// the section does not carry.
    Already,
    /// The path lands on something other than the table or the key this row
    /// edits, so nothing was written anywhere in the document.
    Refused,
}

/// The TOML item a filed value becomes, or [`None`] for one this writer has
/// no arm for.
///
/// A dog pane marks an array and a map read-only, so neither should reach
/// here; writing one as JSON text would put a string in `dogs.toml` where
/// the dog reads a table or a list. A number that is neither an `i64` nor a
/// genuine float (a `u64` above `i64::MAX`) is refused for the same reason:
/// the float it would round to reads back as something else.
fn toml_cell(value: &Value) -> Option<toml_edit::Item> {
    match value {
        Value::Bool(flag) => Some(toml_edit::value(*flag)),
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                Some(toml_edit::value(int))
            } else if number.is_f64() {
                number.as_f64().map(toml_edit::value)
            } else {
                None
            }
        }
        Value::String(text) => Some(toml_edit::value(text.as_str())),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// Sets `cell` at `path`, creating any table on the path the section does not
/// carry yet.
///
/// A parent that is there takes the set where it is: a `[models.worker]`
/// header, a `models.worker.model = ...` dotted key and a
/// `worker = { model = ... }` inline table are all [`toml_edit::TableLike`],
/// and each renders back in the form it came in. A parent the section lacks is
/// created implicit, so the write adds `[models.worker]` and no empty
/// `[models]` header above it.
///
/// [`Landed::Refused`] where the section already holds something else at a
/// step of the path: `models = "old"` there is the operator's value, and a
/// nested row is not licensed to trade it for a table. The same for a leaf
/// that is already a table, which no row of this pane edits.
fn set_at(table: &mut dyn toml_edit::TableLike, path: &[String], cell: toml_edit::Item) -> Landed {
    let Some((key, rest)) = path.split_first() else {
        return Landed::Refused;
    };
    // `entry` rather than `insert`: `insert` re-formats the key it replaces,
    // and that is where a comment the operator wrote above the key goes.
    let entry = table.entry(key).or_insert(toml_edit::Item::None);
    if rest.is_empty() {
        if entry.is_table_like() {
            return Landed::Refused;
        }
        *entry = cell;
        return Landed::Changed;
    }
    if matches!(entry, toml_edit::Item::None) {
        *entry = implicit_table();
    } else if !entry.is_table_like() {
        return Landed::Refused;
    }
    let Some(nested) = entry.as_table_like_mut() else {
        return Landed::Refused;
    };
    set_at(nested, rest, cell)
}

/// Removes the leaf `path` names, and nothing above it.
///
/// [`Landed::Already`] when the leaf, or a table on the way to it, is not in
/// the section: the key is already unset, which is what the edit wanted.
/// [`Landed::Refused`] only when a step on the path is there and holds no
/// keys, so the removal was aimed at something that is not a table.
fn remove_at(table: &mut dyn toml_edit::TableLike, path: &[String]) -> Landed {
    let Some((key, rest)) = path.split_first() else {
        return Landed::Refused;
    };
    if rest.is_empty() {
        return if table.remove(key).is_some() {
            Landed::Changed
        } else {
            Landed::Already
        };
    }
    let Some(item) = table.get_mut(key) else {
        return Landed::Already;
    };
    if matches!(item, toml_edit::Item::None) {
        return Landed::Already;
    }
    let Some(nested) = item.as_table_like_mut() else {
        return Landed::Refused;
    };
    remove_at(nested, rest)
}

/// A table `toml_edit` will not print a header for while it has no values of
/// its own, which is what keeps a created parent from showing as `[models]`.
fn implicit_table() -> toml_edit::Item {
    let mut table = toml_edit::Table::new();
    table.set_implicit(true);
    toml_edit::Item::Table(table)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::fixtures::{bark_pane, field, web};
    use super::*;

    /// A dog whose schema nests one table inside another through a `$ref`
    /// chain, holds a secret beside a plain field in the inner one, and has
    /// a property literally named with a dot.
    fn nested_schema() -> Value {
        json!({
            "properties": {
                "name": { "type": "string" },
                "models": { "$ref": "#/$defs/Models" },
                "a.b": { "type": "string" },
            },
            "$defs": {
                "Models": {
                    "type": "object",
                    "properties": { "worker": { "$ref": "#/$defs/Worker" } },
                },
                "Worker": {
                    "type": "object",
                    "properties": {
                        "model": { "type": "string" },
                        "token": { "type": "string", "x-shep-secret": true },
                    },
                },
            },
        })
    }

    fn nested_pane(section: &str) -> ConfigPane {
        ConfigPane::dog("jobs".into(), None, nested_schema(), section.to_owned())
    }

    /// The same data in the three forms `dogs.toml` allows for a nested
    /// table, each with a comment above the table and one inside it.
    const HEADER: &str = concat!(
        "# which model to run\n",
        "name = \"small\"\n",
        "\n",
        "[models.worker]\n",
        "# the credential\n",
        "token = \"sk-old\"\n",
        "model = \"gpt\"\n",
    );
    const DOTTED: &str = concat!(
        "# which model to run\n",
        "name = \"small\"\n",
        "models.worker.token = \"sk-old\"\n",
        "models.worker.model = \"gpt\"\n",
    );
    const INLINE: &str = concat!(
        "# which model to run\n",
        "name = \"small\"\n",
        "models = { worker = { token = \"sk-old\", model = \"gpt\" } }\n",
    );

    /// The header form with the credential left out, which is how an unset
    /// optional field reads in `dogs.toml`.
    const NO_TOKEN: &str = concat!(
        "# which model to run\n",
        "name = \"small\"\n",
        "\n",
        "[models.worker]\n",
        "model = \"gpt\"\n",
    );

    fn edits_of(pairs: &[(&str, Value)]) -> Edits {
        let mut edits = Edits::default();
        for (key, value) in pairs {
            edits.set(field(key, value.clone()), None);
        }
        edits
    }

    /// Every form takes the set where it is, and neither comment survives
    /// anywhere but where the operator wrote it.
    #[test]
    fn a_nested_set_lands_in_the_form_the_section_already_uses() {
        for (section, want) in [
            (
                HEADER,
                concat!(
                    "# which model to run\n",
                    "name = \"small\"\n",
                    "\n",
                    "[models.worker]\n",
                    "# the credential\n",
                    "token = \"sk-old\"\n",
                    "model = \"large\"\n",
                ),
            ),
            (
                DOTTED,
                concat!(
                    "# which model to run\n",
                    "name = \"small\"\n",
                    "models.worker.token = \"sk-old\"\n",
                    "models.worker.model = \"large\"\n",
                ),
            ),
            (
                INLINE,
                concat!(
                    "# which model to run\n",
                    "name = \"small\"\n",
                    "models = { worker = { token = \"sk-old\", model = \"large\" } }\n",
                ),
            ),
        ] {
            let out = nested_pane(section)
                .edited_section_with(&edits_of(&[("models.worker.model", json!("large"))]))
                .expect("the section parses");
            assert_eq!(out, want, "{section}");
        }
    }

    /// A set into a table the section does not carry writes the deepest
    /// header it needs and nothing above it.
    #[test]
    fn a_set_that_creates_a_table_writes_no_empty_parent_header() {
        let out = nested_pane("name = \"small\"\n")
            .edited_section_with(&edits_of(&[("models.worker.model", json!("large"))]))
            .expect("the section parses");
        assert_eq!(
            out,
            concat!(
                "name = \"small\"\n",
                "\n",
                "[models.worker]\n",
                "model = \"large\"\n",
            )
        );
    }

    /// Key order is the operator's: a set rewrites one line in place, and a
    /// new key joins its table at the end rather than at the top.
    #[test]
    fn key_order_survives_a_set() {
        let out = nested_pane(HEADER)
            .edited_section_with(&edits_of(&[
                ("models.worker.token", json!("sk-new")),
                ("name", json!("large")),
            ]))
            .expect("the section parses");
        assert_eq!(
            out,
            concat!(
                "# which model to run\n",
                "name = \"large\"\n",
                "\n",
                "[models.worker]\n",
                "# the credential\n",
                "token = \"sk-new\"\n",
                "model = \"gpt\"\n",
            )
        );
    }

    /// Two edits to one table are two lines of the same write.
    #[test]
    fn two_edits_in_one_table_both_land() {
        let out = nested_pane(HEADER)
            .edited_section_with(&edits_of(&[
                ("models.worker.model", json!("large")),
                ("models.worker.token", json!("sk-new")),
            ]))
            .expect("the section parses");
        assert_eq!(
            out,
            concat!(
                "# which model to run\n",
                "name = \"small\"\n",
                "\n",
                "[models.worker]\n",
                "# the credential\n",
                "token = \"sk-new\"\n",
                "model = \"large\"\n",
            )
        );
    }

    /// `null` is the leaf's own removal: its sibling and the table holding
    /// both stay, comment included.
    #[test]
    fn a_null_removes_only_its_leaf() {
        for (section, want) in [
            (
                HEADER,
                concat!(
                    "# which model to run\n",
                    "name = \"small\"\n",
                    "\n",
                    "[models.worker]\n",
                    "# the credential\n",
                    "token = \"sk-old\"\n",
                ),
            ),
            (
                DOTTED,
                concat!(
                    "# which model to run\n",
                    "name = \"small\"\n",
                    "models.worker.token = \"sk-old\"\n",
                ),
            ),
            (
                INLINE,
                concat!(
                    "# which model to run\n",
                    "name = \"small\"\n",
                    // The space the removed entry owned goes with it.
                    "models = { worker = { token = \"sk-old\"} }\n",
                ),
            ),
        ] {
            let out = nested_pane(section)
                .edited_section_with(&edits_of(&[("models.worker.model", Value::Null)]))
                .expect("the section parses");
            assert_eq!(out, want, "{section}");
        }
    }

    /// A batch that changed nothing sends nothing: the removal found no
    /// parent, so the section it would have sent is the one it was handed.
    #[test]
    fn a_null_under_an_absent_parent_writes_nothing() {
        assert_eq!(
            nested_pane("name = \"small\"\n")
                .edited_section_with(&edits_of(&[("models.worker.model", Value::Null)])),
            None
        );
    }

    /// An unset field the operator unsets again is a no-op, not a failure: a
    /// refusal here would drop the sibling edit in the same close along with
    /// it.
    #[test]
    fn a_null_on_a_key_the_section_does_not_carry_keeps_the_rest_of_the_batch() {
        let out = nested_pane(NO_TOKEN)
            .edited_section_with(&edits_of(&[
                ("models.worker.token", Value::Null),
                ("name", json!("large")),
            ]))
            .expect("the removal changed nothing, so the batch stands on its edit");
        assert_eq!(
            out,
            concat!(
                "# which model to run\n",
                "name = \"large\"\n",
                "\n",
                "[models.worker]\n",
                "model = \"gpt\"\n",
            )
        );
    }

    /// The same for a parent the section does not carry at all.
    #[test]
    fn a_null_under_an_absent_parent_keeps_the_rest_of_the_batch() {
        let out = nested_pane("name = \"small\"\n")
            .edited_section_with(&edits_of(&[
                ("models.worker.model", Value::Null),
                ("name", json!("large")),
            ]))
            .expect("nothing to remove is not a failure");
        assert_eq!(out, "name = \"large\"\n");
    }

    /// `models = "old"` is the operator's own line, and a row editing a key
    /// inside `models` is not licensed to trade it for a table. Refused whole:
    /// a batch this writer cannot finish sends no part of itself.
    #[test]
    fn a_set_onto_a_scalar_where_the_schema_has_a_table_writes_nothing() {
        for section in [
            "models = \"old\"\nname = \"small\"\n",
            "models.worker = \"old\"\nname = \"small\"\n",
            "models = { worker = \"old\" }\nname = \"small\"\n",
        ] {
            let pane = nested_pane(section);
            assert_eq!(
                pane.edited_section_with(&edits_of(&[("models.worker.model", json!("large"))])),
                None,
                "{section}"
            );
            assert_eq!(
                pane.edited_section_with(&edits_of(&[
                    ("models.worker.model", json!("large")),
                    ("name", json!("large")),
                ])),
                None,
                "{section}"
            );
        }
    }

    /// No row of this pane ends at a table, so this is the writer's own rule
    /// stated where it lives rather than a path an operator reaches.
    #[test]
    fn a_set_never_replaces_a_table_the_operator_wrote() {
        let mut doc: toml_edit::DocumentMut = "[models.worker]\nmodel = \"gpt\"\n"
            .parse()
            .expect("the section parses");
        let path = ["models".to_owned()];
        assert_eq!(
            set_at(doc.as_table_mut(), &path, toml_edit::value("old")),
            Landed::Refused
        );
        assert!(doc.to_string().contains("[models.worker]"), "{doc}");
    }

    /// A property literally named `a.b` is one key, and the section spells
    /// it quoted. The row key carries the quotes `segment` added to it.
    #[test]
    fn a_property_named_with_a_dot_stays_one_key() {
        let pane = nested_pane("\"a.b\" = \"literal\"\nname = \"small\"\n");
        assert_eq!(pane.value("\"a.b\""), "literal");
        let out = pane
            .edited_section_with(&edits_of(&[("\"a.b\"", json!("other"))]))
            .expect("the section parses");
        assert_eq!(out, "\"a.b\" = \"other\"\nname = \"small\"\n");
    }

    /// A number this pane cannot name is refused rather than rounded: a
    /// `u64` above `i64::MAX` reaches `toml_edit` as a float, and a rounded
    /// number in `dogs.toml` is a wrong setting the dog reads back as valid.
    #[test]
    fn a_number_that_is_neither_an_integer_nor_a_float_writes_nothing() {
        let pane = nested_pane("name = \"small\"\n");
        assert_eq!(
            pane.edited_section_with(&edits_of(&[("name", json!(u64::MAX))])),
            None
        );
    }

    /// The pane marks an array and a map read-only, so neither should reach
    /// the writer. JSON text written as a TOML string is a value the dog
    /// cannot read back as the shape it was.
    #[test]
    fn an_array_or_a_map_writes_nothing() {
        let pane = nested_pane("name = \"small\"\n");
        for value in [json!(["a", "b"]), json!({ "model": "gpt" })] {
            assert_eq!(
                pane.edited_section_with(&edits_of(&[("name", value.clone())])),
                None,
                "{value}"
            );
        }
    }

    /// The four shapes the writer does have an arm for, each in its own TOML
    /// type rather than as text.
    #[test]
    fn a_scalar_writes_its_own_toml_type() {
        for (value, want) in [
            (json!(-5), "count = -5\n"),
            (json!(1.5), "count = 1.5\n"),
            (json!(true), "count = true\n"),
            (json!("gpt"), "count = \"gpt\"\n"),
        ] {
            let pane = nested_pane("count = 0\n");
            let out = pane
                .edited_section_with(&edits_of(&[("count", value.clone())]))
                .expect("the section parses");
            assert_eq!(out, want, "{value}");
        }
    }

    /// Re-rendering from parsed values would delete every comment in a file
    /// shep does not author.
    #[test]
    fn an_edited_section_keeps_its_comments_and_changes_one_key() {
        let pane = bark_pane();
        let mut edits = Edits::default();
        edits.set(field("poll", json!("30s")), None);
        let out = pane
            .edited_section_with(&edits)
            .expect("the fixture section parses");
        assert!(out.contains("# how often"), "{out}");
        assert!(out.contains("poll = \"30s\""), "{out}");
        assert!(out.contains("history_bytes = 4096"), "{out}");
        assert!(out.contains("url = \"https://hooks.example/x\""), "{out}");
    }

    /// An empty buffer unsets the key, putting it back on the dog's own
    /// default.
    #[test]
    fn a_null_edit_removes_the_key_from_the_section() {
        let pane = bark_pane();
        let mut edits = Edits::default();
        edits.set(field("history_bytes", Value::Null), None);
        let out = pane
            .edited_section_with(&edits)
            .expect("the fixture section parses");
        assert!(!out.contains("history_bytes"), "{out}");
        assert!(out.contains("# how often"), "{out}");
    }

    /// A dog section takes every filed edit in one write rather than one
    /// write per edit.
    #[test]
    fn a_dog_section_takes_every_filed_edit_in_one_write() {
        let pane = bark_pane();
        let mut edits = Edits::default();
        edits.set(field("sink_timeout", json!("5s")), None);
        edits.set(field("poll", json!("30s")), None);
        let section = pane
            .edited_section_with(&edits)
            .expect("the fixture parses");
        assert!(section.contains("poll = \"30s\""), "{section}");
        assert!(section.contains("sink_timeout = \"5s\""), "{section}");
    }

    /// An env edit has no home in a dog's section and must not silently
    /// become a key in it.
    #[test]
    fn an_env_edit_is_ignored_by_a_dog_section() {
        let pane = bark_pane();
        let mut edits = Edits::default();
        edits.set(
            PaneEdit::SetEnv {
                key: "SECRET".to_owned(),
                value: None,
            },
            None,
        );
        assert_eq!(pane.edited_section_with(&edits), None);
    }

    /// A stray env edit in a set that also carries a field edit must not
    /// revive the old "one bad entry aborts the whole batch" bug: skipping
    /// the env edit and an early return on any non-field entry both leave
    /// `an_env_edit_is_ignored_by_a_dog_section` green, since an env-only
    /// set returns `None` either way. Only a mixed set tells them apart.
    #[test]
    fn a_field_edit_lands_even_when_the_same_batch_carries_an_env_edit() {
        let pane = bark_pane();
        let mut edits = Edits::default();
        edits.set(field("poll", json!("30s")), None);
        edits.set(
            PaneEdit::SetEnv {
                key: "SECRET".to_owned(),
                value: None,
            },
            None,
        );
        let out = pane
            .edited_section_with(&edits)
            .expect("the field edit alone should still write");
        assert!(out.contains("poll = \"30s\""), "{out}");
        assert!(!out.contains("SECRET"), "{out}");
    }

    /// A section here would send a sheep's config out through a dog's door.
    #[test]
    fn a_sheep_pane_has_no_section_to_edit() {
        let pane = ConfigPane::sheep(web());
        let mut edits = Edits::default();
        edits.set(field("cwd", json!("/srv")), None);
        assert_eq!(pane.edited_section_with(&edits), None);
    }
}
