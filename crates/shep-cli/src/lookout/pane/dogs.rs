//! The dogs sub-screen: every dog that can hold a table on this sheep, and
//! whether it does.
//!
//! Opened from a sheep pane's `dogs` row. A dog is listed when its schema
//! answer carries `x-shep-sheep`, set or not, and when the sheep carries a
//! table for it that no schema describes. The second kind is read-only: with
//! no schema nothing says which of its values is a secret, so it shows key
//! names and never a value.

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::super::viewport::Viewport;
use super::{ConfigPane, PaneTarget};

/// What one dog answered when the sub-screen probed it.
///
/// `Debug` is manual (IR-41): a schema answer carries the dog's own
/// defaults, which the secret marker exists to keep off a screen, so it
/// prints only whether there is one.
#[derive(Clone, PartialEq, Eq)]
pub struct SheepDogEntry {
    /// The dog.
    pub name: String,
    /// The adopted binary, or [`None`] for a built-in.
    pub adopted_path: Option<PathBuf>,
    /// Its `x-shep-sheep` value with the root's `$defs` attached, or
    /// [`None`] for a dog that publishes no per-sheep schema.
    pub schema: Option<Value>,
}

impl core::fmt::Debug for SheepDogEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "SheepDogEntry {{ name: {:?}, adopted_path: {:?}, schema: {} }}",
            self.name,
            self.adopted_path,
            self.schema.as_ref().map_or("None", |_| "Some(..)")
        )
    }
}

/// Whether the sheep carries a table for a listed dog, and whether the pane
/// can edit it.
///
/// `Debug` is derived (IR-41): a bare variant name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DogTableState {
    /// A table, and a schema to edit it through.
    Set,
    /// No table yet; the schema is what lists the dog.
    Unset,
    /// A table and no schema: shown by key name, never edited.
    ReadOnly,
}

/// One row of the sub-screen.
///
/// `Debug` is manual (IR-41), for [`SheepDogEntry`]'s reason.
#[derive(Clone, PartialEq, Eq)]
pub struct DogRow {
    entry: SheepDogEntry,
    state: DogTableState,
    keys: Vec<String>,
}

impl core::fmt::Debug for DogRow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "DogRow {{ name: {:?}, state: {:?}, keys: {:?} }}",
            self.entry.name, self.state, self.keys
        )
    }
}

impl DogRow {
    /// The dog.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.entry.name
    }

    /// Set, unset or read-only.
    #[must_use]
    pub fn state(&self) -> DogTableState {
        self.state
    }

    /// The table's top-level key names, in key order. Empty
    /// for an unset row.
    #[must_use]
    pub fn keys(&self) -> &[String] {
        &self.keys
    }

    /// What the probe answered for this dog.
    #[must_use]
    pub fn entry(&self) -> &SheepDogEntry {
        &self.entry
    }
}

/// The sub-screen's state: its rows, the cursor, and at most one armed
/// removal.
///
/// Holds the probe's answers, so a re-read of the sheep rebuilds the rows
/// without probing again. `Debug` is manual (IR-41), for
/// [`SheepDogEntry`]'s reason.
#[derive(Clone, PartialEq, Eq)]
pub struct DogsPane {
    probes: Vec<SheepDogEntry>,
    rows: Vec<DogRow>,
    view: Viewport,
    armed: Option<String>,
}

impl core::fmt::Debug for DogsPane {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "DogsPane {{ rows: {:?}, cursor: {}, armed: {:?} }}",
            self.rows,
            self.view.cursor(),
            self.armed
        )
    }
}

impl DogsPane {
    /// The rows for a sheep carrying `tables`, keyed by dog name, as
    /// `probes` answered. Sorted by name.
    ///
    /// A dog whose probe found a schema is listed set or unset; a table no
    /// schema describes is listed read-only, whether its dog answered
    /// without one or was not probed at all.
    #[must_use]
    pub fn new(tables: &Map<String, Value>, probes: Vec<SheepDogEntry>) -> Self {
        let mut rows: Vec<DogRow> = Vec::new();
        for probe in &probes {
            let table = tables.get(&probe.name);
            let state = match (&probe.schema, table) {
                (Some(_), Some(_)) => DogTableState::Set,
                (Some(_), None) => DogTableState::Unset,
                (None, Some(_)) => DogTableState::ReadOnly,
                (None, None) => continue,
            };
            rows.push(DogRow {
                entry: probe.clone(),
                state,
                keys: key_names(table),
            });
        }
        for (name, table) in tables {
            if probes.iter().any(|probe| &probe.name == name) {
                continue;
            }
            rows.push(DogRow {
                entry: SheepDogEntry {
                    name: name.clone(),
                    adopted_path: None,
                    schema: None,
                },
                state: DogTableState::ReadOnly,
                keys: key_names(Some(table)),
            });
        }
        rows.sort_by(|a, b| a.entry.name.cmp(&b.entry.name));
        Self {
            probes,
            rows,
            view: Viewport::new(),
            armed: None,
        }
    }

    /// The same probe answers over the sheep's current `tables`, the cursor
    /// kept on the dog it was on. A dog that left the list puts the cursor
    /// where that row was. Nothing stays armed.
    #[must_use]
    pub fn refreshed(&self, tables: &Map<String, Value>) -> Self {
        let mut fresh = Self::new(tables, self.probes.clone());
        let on = self.cursor_row().map(DogRow::name);
        let index = on
            .and_then(|name| fresh.rows.iter().position(|row| row.name() == name))
            .unwrap_or_else(|| self.view.cursor());
        fresh.view = self.view.clone();
        let len = fresh.rows.len();
        fresh.view.move_to(index, len);
        fresh
    }

    /// Every row, in display order.
    #[must_use]
    pub fn rows(&self) -> &[DogRow] {
        &self.rows
    }

    /// The row under the cursor, or [`None`] for an empty list.
    #[must_use]
    pub fn cursor_row(&self) -> Option<&DogRow> {
        self.rows.get(self.view.cursor())
    }

    /// The cursor and offset.
    #[must_use]
    pub fn view(&self) -> &Viewport {
        &self.view
    }

    /// The dog whose table an `Enter` would remove, or [`None`].
    #[must_use]
    pub fn armed(&self) -> Option<&str> {
        self.armed.as_deref()
    }

    /// Records the terminal's height, in rows of data.
    pub fn set_rows(&mut self, rows: usize) {
        self.view.set_rows(rows, self.rows.len());
    }

    pub(in crate::lookout) fn move_by(&mut self, delta: isize) {
        self.view.move_by(delta, self.rows.len());
    }

    pub(in crate::lookout) fn move_to_first(&mut self) {
        self.view.move_to(0, self.rows.len());
    }

    pub(in crate::lookout) fn move_to_last(&mut self) {
        let len = self.rows.len();
        self.view.move_to(len.saturating_sub(1), len);
    }

    /// Arms the removal of the table under the cursor. Does nothing, and
    /// answers `false`, on a row the sheep carries no table for.
    pub(in crate::lookout) fn arm_removal(&mut self) -> bool {
        let Some(row) = self.cursor_row() else {
            return false;
        };
        if row.state == DogTableState::Unset {
            return false;
        }
        self.armed = Some(row.name().to_owned());
        true
    }

    /// Clears an armed removal, and says whether one was there.
    pub(in crate::lookout) fn disarm(&mut self) -> bool {
        self.armed.take().is_some()
    }
}

/// A table's top-level key names, or none for no table.
fn key_names(table: Option<&Value>) -> Vec<String> {
    table
        .and_then(Value::as_object)
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default()
}

/// A sheep's `dogs` field as its row draws it: the dog names, comma
/// separated, or `none`. Never a value from a table: without a schema
/// nothing says which of them is a secret.
pub(crate) fn dog_names(dogs: Option<&Value>) -> String {
    let names: Vec<&str> = dogs
        .and_then(Value::as_object)
        .map(|tables| tables.keys().map(String::as_str).collect())
        .unwrap_or_default();
    if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    }
}

impl ConfigPane {
    /// The open dogs sub-screen, or [`None`].
    #[must_use]
    pub fn dogs(&self) -> Option<&DogsPane> {
        self.dogs.as_deref()
    }

    pub(in crate::lookout) fn dogs_mut(&mut self) -> Option<&mut DogsPane> {
        self.dogs.as_deref_mut()
    }

    /// Opens the dogs sub-screen over this sheep's tables, as `probes`
    /// answered. Does nothing on a pane that is not a sheep's.
    pub(in crate::lookout) fn open_dogs(&mut self, probes: Vec<SheepDogEntry>) {
        if !matches!(self.target, PaneTarget::Sheep { .. }) {
            return;
        }
        self.dogs = Some(Box::new(DogsPane::new(&self.sheep_tables(), probes)));
    }

    /// Closes it, leaving the field list up.
    pub(in crate::lookout) fn close_dogs(&mut self) {
        self.dogs = None;
    }

    /// Carries a previous pane's sub-screen onto this one's tables, for a
    /// re-read that rebuilt the pane under it.
    pub(in crate::lookout) fn adopt_dogs(&mut self, previous: &DogsPane) {
        self.dogs = Some(Box::new(previous.refreshed(&self.sheep_tables())));
    }

    /// `dog`'s table on this sheep, or an empty one when it carries none.
    #[must_use]
    pub(in crate::lookout) fn sheep_table(&self, dog: &str) -> Map<String, Value> {
        self.values
            .get("dogs")
            .and_then(|dogs| dogs.get(dog))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }

    fn sheep_tables(&self) -> Map<String, Value> {
        self.values
            .get("dogs")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use shep_core::config::AppConfig;
    use shep_core::protocol::SheepConfigView;

    use super::*;
    use crate::lookout::view::fixtures::{plain, render_all};
    use crate::lookout::view::pane::pane_lines;

    /// `web` carrying a `jobs` table with a credential in it and a `legacy`
    /// table for a dog that publishes no sheep schema.
    fn tables() -> Map<String, Value> {
        json!({
            "jobs": { "concurrency": 2, "token": "sk-live-51Hx9Qa" },
            "legacy": { "url": "https://ops:hunter2@example.test", "retries": 3 },
        })
        .as_object()
        .cloned()
        .expect("an object")
    }

    fn probe(name: &str, schema: bool) -> SheepDogEntry {
        SheepDogEntry {
            name: name.to_owned(),
            adopted_path: Some(PathBuf::from(format!("/opt/{name}"))),
            schema: schema.then(|| json!({ "type": "object", "properties": {} })),
        }
    }

    fn web_with_tables() -> ConfigPane {
        let mut config = AppConfig {
            name: "web".into(),
            ..AppConfig::default()
        };
        for (name, table) in tables() {
            let table = table.as_object().cloned().expect("a table");
            config.dogs.insert(name, table.into());
        }
        ConfigPane::sheep(SheepConfigView::new(config, Vec::new(), Vec::new()))
    }

    #[test]
    fn the_dogs_row_names_the_dogs_and_never_a_value() {
        let pane = web_with_tables();
        assert_eq!(pane.value("dogs"), "jobs, legacy");
        assert_eq!(pane.display_value("dogs"), "jobs, legacy");
        let mut all = pane.clone();
        let mut text = String::new();
        for digit in 1..=9 {
            all.set_group(digit);
            text.push_str(&render_all(&pane_lines(&all, plain(), 200, 0)));
        }
        assert!(text.contains("jobs, legacy"), "{text}");
        assert!(!text.contains("sk-live-51Hx9Qa"), "{text}");
        assert!(!text.contains("hunter2"), "{text}");
    }

    #[test]
    fn a_sheep_with_no_tables_shows_none() {
        let view = SheepConfigView::new(
            AppConfig {
                name: "web".into(),
                ..AppConfig::default()
            },
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(ConfigPane::sheep(view).value("dogs"), "none");
    }

    #[test]
    fn the_list_marks_set_unset_and_read_only_and_sorts_by_name() {
        let dogs = DogsPane::new(
            &tables(),
            vec![
                probe("jobs", true),
                probe("deploy", true),
                probe("legacy", false),
                probe("metrics", false),
            ],
        );
        let listed: Vec<(&str, DogTableState)> = dogs
            .rows()
            .iter()
            .map(|row| (row.name(), row.state()))
            .collect();
        assert_eq!(
            listed,
            [
                ("deploy", DogTableState::Unset),
                ("jobs", DogTableState::Set),
                ("legacy", DogTableState::ReadOnly),
            ],
            "a dog with neither a schema nor a table is not listed"
        );
    }

    #[test]
    fn a_table_whose_dog_was_never_probed_is_read_only() {
        let dogs = DogsPane::new(&tables(), vec![probe("jobs", true)]);
        let legacy = &dogs.rows()[1];
        assert_eq!(legacy.name(), "legacy");
        assert_eq!(legacy.state(), DogTableState::ReadOnly);
        assert_eq!(legacy.entry().schema, None);
    }

    #[test]
    fn a_read_only_row_carries_key_names_and_no_value() {
        let dogs = DogsPane::new(&tables(), vec![probe("legacy", false)]);
        let legacy = &dogs.rows()[1];
        assert_eq!(legacy.keys(), ["retries", "url"]);
        let debug = format!("{dogs:?}");
        assert!(!debug.contains("hunter2"), "{debug}");
        assert!(!debug.contains("sk-live-51Hx9Qa"), "{debug}");
    }

    #[test]
    fn an_empty_list_moves_nowhere_and_arms_nothing() {
        let mut dogs = DogsPane::new(&Map::new(), Vec::new());
        dogs.move_by(1);
        dogs.move_to_last();
        dogs.move_by(-1);
        assert_eq!(dogs.view().cursor(), 0);
        assert!(dogs.cursor_row().is_none());
        assert!(!dogs.arm_removal());
    }

    #[test]
    fn arming_a_removal_on_an_unset_row_does_nothing() {
        let mut dogs = DogsPane::new(&tables(), vec![probe("deploy", true)]);
        assert_eq!(dogs.cursor_row().map(DogRow::name), Some("deploy"));
        assert!(!dogs.arm_removal());
        assert_eq!(dogs.armed(), None);
        dogs.move_by(1);
        assert_eq!(
            dogs.cursor_row().map(DogRow::state),
            Some(DogTableState::ReadOnly)
        );
        assert!(dogs.arm_removal(), "a read-only table can still be removed");
        assert_eq!(dogs.armed(), Some("jobs"));
        assert!(dogs.disarm());
        assert_eq!(dogs.armed(), None);
    }

    /// A table added above the cursor moves its dog down a row, so a
    /// cursor carried by index would land on a neighbour.
    #[test]
    fn a_refresh_keeps_the_cursor_on_its_dog() {
        let probes = vec![probe("deploy", true), probe("jobs", true)];
        let mut dogs = DogsPane::new(&tables(), probes);
        dogs.move_by(1);
        assert_eq!(dogs.cursor_row().map(DogRow::name), Some("jobs"));
        let mut more = tables();
        more.insert("alpha".into(), json!({ "on": true }));
        let fresh = dogs.refreshed(&more);
        assert_eq!(fresh.cursor_row().map(DogRow::name), Some("jobs"));

        dogs.move_to_last();
        let mut fewer = tables();
        fewer.remove("legacy");
        let fresh = dogs.refreshed(&fewer);
        assert_eq!(
            fresh.cursor_row().map(DogRow::name),
            Some("jobs"),
            "a dog that left puts the cursor where its row was"
        );
    }

    /// A schema answer carries a dog's own defaults (IR-41).
    #[test]
    fn the_debug_of_an_entry_names_no_schema() {
        let entry = SheepDogEntry {
            name: "jobs".into(),
            adopted_path: None,
            schema: Some(json!({ "default": "sk-live-51Hx9Qa" })),
        };
        assert_eq!(
            format!("{entry:?}"),
            r#"SheepDogEntry { name: "jobs", adopted_path: None, schema: Some(..) }"#
        );
    }
}
