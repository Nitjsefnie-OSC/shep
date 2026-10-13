//! The rows under an array or map of tables on a dog's pane.
//!
//! A field of [`FieldKind::Entries`] is one row, and the entries under it
//! are rows the schema alone cannot name: how many there are and what each
//! is called come from the values. They are built once, when the pane is,
//! into an arena the pane's rows index, the way its outline indexes tables.
//! Every row here is read-only.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use super::super::field::{
    Child, Entries, Field, FieldKind, FieldSet, Flattened, flatten_values, segment,
};
use super::PaneRow;
use super::fields::{entries_text, render_json};

/// Every row under an [`FieldKind::Entries`] field of one pane.
///
/// No `Debug` (IR-41): the rows hold the values the pane withholds.
#[derive(Clone, Default)]
pub(super) struct EntryRows {
    /// The item schema of each entries field, by the field's dotted key,
    /// kept so [`Self::rebuilt`] can read new values.
    shapes: Arc<BTreeMap<String, Flattened>>,
    rows: Vec<EntryRow>,
    /// The entries directly under each [`FieldKind::Entries`] field, by the
    /// field's index, in the order they are drawn.
    roots: BTreeMap<usize, Vec<usize>>,
}

/// One row under an [`FieldKind::Entries`] field.
///
/// No `Debug`, for the reason [`EntryRows`] gives.
#[derive(Clone)]
pub(in crate::lookout) struct EntryRow {
    /// How many rows it is indented under, the same unit as a field's.
    pub(in crate::lookout) depth: usize,
    /// The entry's key, name or `#<index>`; for anything inside an entry,
    /// the property's last segment.
    pub(in crate::lookout) label: String,
    /// What the row is.
    pub(in crate::lookout) body: EntryBody,
    /// The row it sits under, for `←`.
    pub(super) parent: PaneRow,
}

/// What an [`EntryRow`] is.
#[derive(Clone)]
pub(in crate::lookout) enum EntryBody {
    /// An entry, or a table inside one. Opens and closes.
    Node {
        /// Its key in the pane's open set.
        path: String,
        /// The arena rows directly under it, in order.
        children: Vec<usize>,
        /// Whether it is an entry, and not a table inside one.
        entry: bool,
    },
    /// One of the item schema's own properties.
    Leaf {
        /// The property, for its type.
        field: Box<Field>,
        /// The value as a row draws it: a count for a nested
        /// [`FieldKind::Entries`].
        text: String,
        /// Whether the row draws `<set>` in place of `text`.
        masked: bool,
    },
}

impl EntryRows {
    /// The rows under every [`FieldKind::Entries`] field in `fields`.
    ///
    /// `paths` and `values` are the pane's own, keyed by the fields' dotted
    /// keys; `shapes` is [`Flattened::shapes`].
    pub(super) fn build(
        fields: &FieldSet,
        paths: &BTreeMap<String, Vec<String>>,
        values: &Map<String, Value>,
        shapes: BTreeMap<String, Flattened>,
    ) -> Self {
        Self::built(fields, paths, values, Arc::new(shapes))
    }

    /// These rows over `values`, the shapes unchanged.
    pub(super) fn rebuilt(
        &self,
        fields: &FieldSet,
        paths: &BTreeMap<String, Vec<String>>,
        values: &Map<String, Value>,
    ) -> Self {
        Self::built(fields, paths, values, Arc::clone(&self.shapes))
    }

    fn built(
        fields: &FieldSet,
        paths: &BTreeMap<String, Vec<String>>,
        values: &Map<String, Value>,
        shapes: Arc<BTreeMap<String, Flattened>>,
    ) -> Self {
        let mut built = Self {
            shapes: Arc::clone(&shapes),
            ..Self::default()
        };
        for (index, field) in fields.fields().iter().enumerate() {
            let FieldKind::Entries(kind) = field.kind else {
                continue;
            };
            let (Some(shape), Some(path)) = (shapes.get(&field.key), paths.get(&field.key)) else {
                continue;
            };
            let at = Where {
                kind,
                shape,
                base: &field.key,
                depth: path.len(),
                parent: PaneRow::Field(index),
            };
            let roots = built.push_entries(&at, values.get(&field.key));
            built.roots.insert(index, roots);
        }
        built
    }

    /// Row `index`.
    pub(super) fn row(&self, index: usize) -> Option<&EntryRow> {
        self.rows.get(index)
    }

    /// The entries directly under field `field`, or [`None`] when it is not
    /// an entries field with a shape.
    pub(super) fn roots(&self, field: usize) -> Option<&[usize]> {
        self.roots.get(&field).map(Vec::as_slice)
    }

    /// The open-set key of every entry and table inside one.
    pub(super) fn node_paths(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().filter_map(|row| match &row.body {
            EntryBody::Node { path, .. } => Some(path.as_str()),
            EntryBody::Leaf { .. } => None,
        })
    }

    fn push_entries(&mut self, at: &Where<'_>, value: Option<&Value>) -> Vec<usize> {
        let mut entries: Vec<(String, String, &Value)> = Vec::new();
        match (at.kind, value) {
            (Entries::Array, Some(Value::Array(items))) => {
                for (index, item) in items.iter().enumerate() {
                    let path = format!("{}[{index}]", at.base);
                    entries.push((path, array_label(at.shape, item, index), item));
                }
            }
            (Entries::Map, Some(Value::Object(map))) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for key in keys {
                    let quoted = serde_json::to_string(key).unwrap_or_default();
                    let path = format!("{}[{quoted}]", at.base);
                    entries.push((path, plain(key), &map[key]));
                }
            }
            _ => {}
        }
        let mut roots = Vec::new();
        for (path, label, item) in entries {
            let index = self.rows.len();
            self.rows.push(EntryRow {
                depth: at.depth,
                label,
                body: EntryBody::Node {
                    path: path.clone(),
                    children: Vec::new(),
                    entry: true,
                },
                parent: at.parent,
            });
            let table = item.as_object().cloned().unwrap_or_default();
            let values = flatten_values(&table, &at.shape.paths);
            let children = self.push_children(
                at.shape,
                &values,
                &at.shape.outline.roots,
                &Under {
                    depth: at.depth,
                    prefix: &path,
                    parent: PaneRow::Entry(index),
                },
            );
            self.set_children(index, children);
            roots.push(index);
        }
        roots
    }

    fn push_children(
        &mut self,
        shape: &Flattened,
        values: &Map<String, Value>,
        children: &[Child],
        under: &Under<'_>,
    ) -> Vec<usize> {
        let mut pushed = Vec::new();
        for child in children {
            let index = self.rows.len();
            match *child {
                Child::Leaf(field) => {
                    let Some(field) = shape.fields.fields().get(field) else {
                        continue;
                    };
                    let Some(path) = shape.paths.get(&field.key) else {
                        continue;
                    };
                    let value = values.get(&field.key);
                    let counted = matches!(field.kind, FieldKind::Entries(_));
                    self.rows.push(EntryRow {
                        depth: under.depth + path.len(),
                        label: path.last().map(|last| segment(last)).unwrap_or_default(),
                        body: EntryBody::Leaf {
                            field: Box::new(field.clone()),
                            text: if counted {
                                entries_text(value)
                            } else {
                                render_json(value.unwrap_or(&Value::Null))
                            },
                            masked: field.secret && !counted,
                        },
                        parent: under.parent,
                    });
                }
                Child::Table(table) => {
                    let Some(table) = shape.outline.tables.get(table) else {
                        continue;
                    };
                    let path = format!("{}.{}", under.prefix, table.path);
                    self.rows.push(EntryRow {
                        depth: under.depth + 1 + table.depth,
                        label: table.key.clone(),
                        body: EntryBody::Node {
                            path: path.clone(),
                            children: Vec::new(),
                            entry: false,
                        },
                        parent: under.parent,
                    });
                    let below = self.push_children(
                        shape,
                        values,
                        &table.children,
                        &Under {
                            depth: under.depth,
                            prefix: &path,
                            parent: PaneRow::Entry(index),
                        },
                    );
                    self.set_children(index, below);
                }
            }
            pushed.push(index);
        }
        pushed
    }

    fn set_children(&mut self, index: usize, below: Vec<usize>) {
        if let Some(EntryRow {
            body: EntryBody::Node { children, .. },
            ..
        }) = self.rows.get_mut(index)
        {
            *children = below;
        }
    }
}

/// What [`EntryRows::push_entries`] needs to know about the field it builds
/// under.
struct Where<'a> {
    kind: Entries,
    shape: &'a Flattened,
    /// The field's dotted key, which its entries' paths start with.
    base: &'a str,
    /// The entries' depth.
    depth: usize,
    parent: PaneRow,
}

/// Where [`EntryRows::push_children`] is in an entry.
struct Under<'a> {
    /// The entry's depth.
    depth: usize,
    /// The path of the node the rows sit in.
    prefix: &'a str,
    parent: PaneRow,
}

/// `text` with each control character as a space, so a label an operator's
/// data supplies cannot break the line it is drawn on.
fn plain(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// The first of `name` and `id` that the item schema declares as a
/// non-secret string and the entry holds, else `#<index>`. A property marked
/// secret never labels a row.
fn array_label(shape: &Flattened, item: &Value, index: usize) -> String {
    ["name", "id"]
        .into_iter()
        .find_map(|key| {
            let field = shape.fields.by_key(key)?;
            if field.secret || !matches!(field.kind, FieldKind::Text | FieldKind::Suggested(_)) {
                return None;
            }
            item.get(key)?.as_str().filter(|text| !text.is_empty())
        })
        .map_or_else(|| format!("#{index}"), plain)
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{entries_pane, entries_schema};
    use super::super::{ConfigPane, PaneRow};
    use super::EntryBody;
    use serde_json::json;

    /// The count a row reads, never the value: `clients` holds a credential.
    #[test]
    fn an_entries_row_reads_as_a_count() {
        let section =
            |text: &str| ConfigPane::dog("paddock".into(), None, entries_schema(), text.to_owned());
        let one = section("[[clients]]\nkey = \"hunter2\"\n");
        let none = section("clients = []\n");
        let absent = section("retries = 3\n");
        let table = |value: serde_json::Value| {
            let table = json!({ "clients": value });
            ConfigPane::sheep_dog(
                "web".into(),
                "paddock".into(),
                &entries_schema(),
                table.as_object().cloned().expect("a table"),
            )
        };
        assert_eq!(entries_pane().value("clients"), "3 entries");
        assert_eq!(one.value("clients"), "1 entry");
        assert_eq!(none.value("clients"), "0 entries");
        assert_eq!(absent.value("clients"), "(unset)");
        assert_eq!(table(json!(null)).value("clients"), "(unset)");
        assert_eq!(entries_pane().value("models"), "2 entries");
        assert_eq!(entries_pane().display_value("clients"), "3 entries");
    }

    /// The labels of every entry directly under `key`, once it is open.
    fn labels(pane: &mut ConfigPane, key: &str) -> Vec<String> {
        pane.move_to_key(key);
        pane.open_table();
        pane.rows()
            .into_iter()
            .filter_map(|row| match row {
                PaneRow::Entry(index) => pane.entry_row(index).filter(|entry| entry.depth == 1),
                _ => None,
            })
            .map(|entry| entry.label.clone())
            .collect()
    }

    #[test]
    fn a_map_is_labelled_by_its_keys_and_an_array_by_name_then_id_then_its_index() {
        assert_eq!(labels(&mut entries_pane(), "models"), ["large", "small"]);
        assert_eq!(
            labels(&mut entries_pane(), "clients"),
            ["alpha", "b-2", "#2"]
        );
    }

    #[test]
    fn a_secret_name_never_labels_an_entry() {
        let schema = json!({
            "properties": { "hosts": { "type": "array", "items": { "$ref": "#/$defs/Host" } } },
            "$defs": { "Host": { "type": "object", "properties": {
                "name": { "type": "string", "x-shep-secret": true },
            } } },
        });
        let mut pane = ConfigPane::dog(
            "paddock".into(),
            None,
            schema,
            "[[hosts]]\nname = \"hunter2\"\n".to_owned(),
        );
        assert_eq!(labels(&mut pane, "hosts"), ["#0"]);
    }

    #[test]
    fn an_entries_field_inside_an_entry_is_a_count_row_with_no_children() {
        let schema = json!({
            "properties": { "clients": { "type": "array", "items": { "$ref": "#/$defs/Client" } } },
            "$defs": {
                "Client": { "type": "object", "properties": {
                    "peers": { "type": "array", "items": { "$ref": "#/$defs/Peer" } },
                } },
                "Peer": { "type": "object", "properties": { "key": { "type": "string" } } },
            },
        });
        let mut pane = ConfigPane::dog(
            "paddock".into(),
            None,
            schema,
            "[[clients]]\n[[clients.peers]]\nkey = \"a\"\n[[clients.peers]]\nkey = \"b\"\n"
                .to_owned(),
        );
        pane.move_to_key("clients");
        pane.open_table();
        pane.move_by(1);
        pane.open_table();
        let rows = pane.rows();
        assert_eq!(rows.len(), 3, "clients, #0 and peers: {rows:?}");
        let Some(PaneRow::Entry(peers)) = rows.last().copied() else {
            panic!("the last row is peers: {rows:?}");
        };
        let peers = pane.entry_row(peers).expect("the row exists");
        assert!(
            matches!(&peers.body, EntryBody::Leaf { text, .. } if text == "2 entries"),
            "a count and not a node"
        );
    }
}
