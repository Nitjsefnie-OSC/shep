//! The tables of a dog's pane: which are open, and the keys that open and
//! close them.
//!
//! A dog's settings are flat rows keyed by a dotted path, and nesting lives
//! in the pane's outline. A table is a row that opens and closes with its
//! keys under it. [`ConfigPane::rows`] walks the outline and these methods
//! move through it.

use std::collections::BTreeSet;

use super::super::field::{Child, segment};
use super::{ConfigPane, PaneRow, PaneTarget};

/// How a row of a dog's pane is drawn in the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::lookout) struct TreeRow {
    /// How many tables the row sits inside.
    pub(in crate::lookout) depth: usize,
    /// The last segment of the row's key, quoted the way a dotted key
    /// quotes it.
    pub(in crate::lookout) label: String,
    /// Whether the row is a table, and then whether it is open.
    pub(in crate::lookout) open: Option<bool>,
}

impl ConfigPane {
    /// Whether the rows nest in tables: a dog's section, or a dog's table on
    /// a sheep.
    pub(in crate::lookout) fn is_tree(&self) -> bool {
        matches!(
            self.target,
            PaneTarget::Dog { .. } | PaneTarget::SheepDog { .. }
        )
    }

    /// `children` and, for each open table among them, what sits under it,
    /// pushed onto `rows` in order.
    pub(super) fn push_tree_rows(&self, children: &[Child], rows: &mut Vec<PaneRow>) {
        for child in children {
            match *child {
                Child::Leaf(index) => rows.push(PaneRow::Field(index)),
                Child::Table(index) => {
                    rows.push(PaneRow::Table(index));
                    if let Some(table) = self.outline.tables.get(index)
                        && self.open.contains(&table.path)
                    {
                        self.push_tree_rows(&table.children, rows);
                    }
                }
            }
        }
    }

    /// How `row` is drawn in the tree, or [`None`] for a pane that is not
    /// one and for a row it does not hold.
    #[must_use]
    pub(in crate::lookout) fn tree_row(&self, row: PaneRow) -> Option<TreeRow> {
        if !self.is_tree() {
            return None;
        }
        match row {
            PaneRow::Table(index) => {
                let table = self.outline.tables.get(index)?;
                Some(TreeRow {
                    depth: table.depth,
                    label: table.key.clone(),
                    open: Some(self.open.contains(&table.path)),
                })
            }
            PaneRow::Field(index) => {
                let key = &self.fields.fields().get(index)?.key;
                let paths = match &self.dog_table {
                    Some(state) => &state.paths,
                    None => &self.section_paths,
                };
                let path = paths.get(key)?;
                Some(TreeRow {
                    depth: path.len().saturating_sub(1),
                    label: segment(path.last()?),
                    open: None,
                })
            }
            PaneRow::Env(_) | PaneRow::AddEnv => None,
        }
    }

    /// The full dotted key of table `index`.
    #[must_use]
    pub(in crate::lookout) fn table_path(&self, index: usize) -> Option<&str> {
        self.outline
            .tables
            .get(index)
            .map(|table| table.path.as_str())
    }

    /// The tables drawn open, by their dotted path.
    #[must_use]
    pub(in crate::lookout) fn open_tables(&self) -> &BTreeSet<String> {
        &self.open
    }

    /// Replaces the open set with a previous pane's, keeping only the tables
    /// this pane's outline still has, so a refresh does not close what the
    /// operator opened and does not hold a table the schema dropped.
    pub(in crate::lookout) fn adopt_open_tables(&mut self, open: &BTreeSet<String>) {
        self.open = self
            .outline
            .tables
            .iter()
            .filter(|table| open.contains(&table.path))
            .map(|table| table.path.clone())
            .collect();
        let len = self.rows().len();
        self.view.clamp(len);
    }

    /// Whether the cursor is on a nested table's row.
    #[must_use]
    pub(in crate::lookout) fn cursor_on_table(&self) -> bool {
        matches!(self.cursor(), Some(PaneRow::Table(_)))
    }

    /// `→`: opens the table under the cursor. Does nothing on an open table,
    /// on a leaf, or on a pane with no tables.
    pub(in crate::lookout) fn open_table(&mut self) {
        if let Some(PaneRow::Table(index)) = self.cursor() {
            self.set_open(index, true);
        }
    }

    /// `←`: closes the open table under the cursor. On a leaf or a closed
    /// table it moves the cursor to the row of the table it sits in instead,
    /// and does nothing at the top level.
    pub(in crate::lookout) fn close_table(&mut self) {
        let Some(row) = self.cursor().filter(|_| self.is_tree()) else {
            return;
        };
        if let PaneRow::Table(index) = row
            && self.is_open(index)
        {
            self.set_open(index, false);
            return;
        }
        let parent = match row {
            PaneRow::Table(index) => self.outline.tables.get(index).and_then(|t| t.parent),
            PaneRow::Field(index) => self
                .outline
                .tables
                .iter()
                .position(|table| table.children.contains(&Child::Leaf(index))),
            PaneRow::Env(_) | PaneRow::AddEnv => None,
        };
        if let Some(parent) = parent {
            self.move_to_row(PaneRow::Table(parent));
        }
    }

    /// `Enter` on a table row: opens a closed table and closes an open one.
    /// Returns whether the cursor was on a table.
    pub(in crate::lookout) fn toggle_table(&mut self) -> bool {
        let Some(PaneRow::Table(index)) = self.cursor() else {
            return false;
        };
        self.set_open(index, !self.is_open(index));
        true
    }

    /// Opens every table `key` sits under, the way an operator opens them
    /// down to a field before walking onto it.
    #[cfg(test)]
    pub(super) fn open_down_to(&mut self, key: &str) {
        let above: Vec<String> = self
            .outline
            .tables
            .iter()
            .filter(|table| key.starts_with(&format!("{}.", table.path)))
            .map(|table| table.path.clone())
            .collect();
        self.open.extend(above);
    }

    fn is_open(&self, index: usize) -> bool {
        self.outline
            .tables
            .get(index)
            .is_some_and(|table| self.open.contains(&table.path))
    }

    /// Opens or closes table `index`. The cursor stays on the row it was on,
    /// or on the table's own row when closing hid that row.
    fn set_open(&mut self, index: usize, open: bool) {
        let Some(table) = self.outline.tables.get(index) else {
            return;
        };
        let held = self.cursor();
        if open {
            self.open.insert(table.path.clone());
        } else {
            self.open.remove(&table.path);
        }
        let rows = self.rows();
        let target = held
            .filter(|row| rows.contains(row))
            .unwrap_or(PaneRow::Table(index));
        self.move_to_row(target);
    }

    fn move_to_row(&mut self, row: PaneRow) {
        let rows = self.rows();
        if let Some(position) = rows.iter().position(|candidate| *candidate == row) {
            self.view.move_to(position, rows.len());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{field, tree_pane, tree_schema, tree_section, web};
    use super::*;
    use crate::lookout::edits::Edits;
    use crate::lookout::pane::SectionWrite;
    use serde_json::json;

    /// A row as the tree draws it: indent, then the label, a table with its
    /// `+` or `-` in front.
    fn drawn(pane: &ConfigPane) -> Vec<String> {
        pane.rows()
            .into_iter()
            .map(|row| {
                let tree = pane.tree_row(row).expect("a tree row");
                let mark = match tree.open {
                    Some(true) => "- ",
                    Some(false) => "+ ",
                    None => "",
                };
                format!("{}{mark}{}", "  ".repeat(tree.depth), tree.label)
            })
            .collect()
    }

    fn go_to(pane: &mut ConfigPane, label: &str, nth: usize) {
        let position = drawn(pane)
            .iter()
            .enumerate()
            .filter(|(_, row)| row.trim_start_matches([' ', '+', '-']) == label)
            .map(|(position, _)| position)
            .nth(nth)
            .expect(label);
        let len = pane.rows().len();
        pane.view.move_to(position, len);
    }

    #[test]
    fn a_new_pane_opens_the_top_level_tables_and_nothing_deeper() {
        let pane = tree_pane();
        assert_eq!(
            drawn(&pane),
            [
                "- agents",
                "  model",
                "  retries",
                "- budget",
                "  cap",
                "  model",
                "- deep",
                "  + inner",
                "name",
            ]
        );
    }

    #[test]
    fn right_opens_the_table_under_the_cursor() {
        let mut pane = tree_pane();
        go_to(&mut pane, "inner", 0);
        pane.open_table();
        assert!(drawn(&pane).contains(&"    leaf".to_owned()));
        assert_eq!(pane.cursor(), Some(PaneRow::Table(3)), "the cursor stays");
    }

    #[test]
    fn left_closes_an_open_table_and_the_cursor_stays_on_its_row() {
        let mut pane = tree_pane();
        go_to(&mut pane, "agents", 0);
        pane.close_table();
        assert_eq!(drawn(&pane)[..2], ["+ agents", "- budget"]);
        assert_eq!(pane.cursor(), Some(PaneRow::Table(0)));
    }

    #[test]
    fn left_on_a_leaf_moves_to_its_parents_row() {
        let mut pane = tree_pane();
        go_to(&mut pane, "cap", 0);
        pane.close_table();
        assert_eq!(pane.cursor(), Some(PaneRow::Table(1)));
        assert_eq!(pane.rows().len(), 9, "nothing closed");
    }

    #[test]
    fn left_on_a_closed_table_moves_to_its_parents_row() {
        let mut pane = tree_pane();
        go_to(&mut pane, "inner", 0);
        pane.close_table();
        assert_eq!(pane.cursor(), Some(PaneRow::Table(2)), "deep");
    }

    #[test]
    fn left_at_the_top_level_does_nothing() {
        let mut pane = tree_pane();
        go_to(&mut pane, "name", 0);
        let before = pane.cursor();
        pane.close_table();
        assert_eq!(pane.cursor(), before);
        go_to(&mut pane, "agents", 0);
        pane.close_table();
        let closed = pane.cursor();
        pane.close_table();
        assert_eq!(
            pane.cursor(),
            closed,
            "a closed top-level table has no parent"
        );
    }

    #[test]
    fn enter_toggles_a_table() {
        let mut pane = tree_pane();
        go_to(&mut pane, "budget", 0);
        assert!(pane.toggle_table());
        assert_eq!(pane.rows().len(), 7);
        assert!(pane.toggle_table());
        assert_eq!(pane.rows().len(), 9);
    }

    #[test]
    fn toggling_a_leaf_is_not_a_toggle() {
        let mut pane = tree_pane();
        go_to(&mut pane, "name", 0);
        assert!(!pane.toggle_table());
    }

    /// A table closed from beneath its cursor, as a refresh could, leaves the
    /// cursor on the table's own row.
    #[test]
    fn closing_a_table_that_holds_the_cursor_puts_it_on_the_table_row() {
        let mut pane = tree_pane();
        go_to(&mut pane, "cap", 0);
        pane.set_open(1, false);
        assert_eq!(pane.cursor(), Some(PaneRow::Table(1)));
    }

    #[test]
    fn closing_a_table_above_the_cursor_keeps_the_cursor_on_its_row() {
        let mut pane = tree_pane();
        go_to(&mut pane, "name", 0);
        pane.set_open(0, false);
        assert_eq!(pane.cursor(), Some(PaneRow::Field(5)), "still `name`");
    }

    #[test]
    fn two_tables_that_each_hold_model_give_two_rows_drawn_as_model() {
        let pane = tree_pane();
        let models: Vec<String> = drawn(&pane)
            .into_iter()
            .filter(|row| row.trim() == "model")
            .collect();
        assert_eq!(models, ["  model", "  model"]);
        let keys: Vec<&str> = pane
            .rows()
            .into_iter()
            .filter_map(|row| match row {
                PaneRow::Field(index) => Some(pane.fields().fields()[index].key.as_str()),
                _ => None,
            })
            .filter(|key| key.ends_with("model"))
            .collect();
        assert_eq!(keys, ["agents.model", "budget.model"]);
    }

    #[test]
    fn adopting_an_open_set_keeps_the_tables_the_schema_still_has() {
        let mut previous = tree_pane();
        go_to(&mut previous, "inner", 0);
        previous.open_table();
        previous.set_open(0, false);
        let carried = previous.open_tables().clone();
        assert!(carried.contains("deep.inner") && !carried.contains("agents"));

        let mut fresh = tree_pane();
        fresh.adopt_open_tables(&carried);
        assert_eq!(fresh.open_tables(), &carried);
    }

    #[test]
    fn adopting_an_open_set_drops_a_table_the_schema_lost() {
        let mut carried = tree_pane().open_tables().clone();
        carried.insert("gone".to_owned());
        carried.insert("deep.gone".to_owned());
        let mut fresh = tree_pane();
        fresh.adopt_open_tables(&carried);
        assert!(!fresh.open_tables().contains("gone"));
        assert!(!fresh.open_tables().contains("deep.gone"));
        assert!(fresh.open_tables().contains("deep"));
    }

    #[test]
    fn a_sheep_pane_has_no_tree_and_no_open_tables() {
        let mut pane = ConfigPane::sheep(web());
        let before = pane.cursor();
        pane.open_table();
        pane.close_table();
        assert!(!pane.toggle_table());
        assert_eq!(pane.cursor(), before);
        assert!(pane.tree_row(PaneRow::Field(0)).is_none());
        assert!(pane.open_tables().is_empty());
    }

    /// An edit to `budget.model` lands on `[budget]` and leaves
    /// `[agents] model` alone, though both rows draw as `model`.
    #[test]
    fn an_edit_to_one_of_two_model_rows_writes_only_its_own_path_into_the_section() {
        let pane = tree_pane();
        let mut edits = Edits::default();
        edits.set(field("budget.model", json!("huge")), None);
        let SectionWrite::Written(written) = pane.edited_section_with(&edits) else {
            panic!("the edit lands");
        };
        assert_eq!(
            written.as_str(),
            tree_section().replace("model = \"large\"", "model = \"huge\"")
        );
    }

    /// The same two rows over a sheep's table: the write is the table with
    /// one leaf changed.
    #[test]
    fn an_edit_to_one_of_two_model_rows_writes_only_its_own_path_into_the_table() {
        let table = json!({
            "name": "x",
            "agents": { "model": "small" },
            "budget": { "model": "large", "cap": 3 },
        });
        let pane = ConfigPane::sheep_dog(
            "web".into(),
            "tree".into(),
            &tree_schema(),
            table.as_object().cloned().expect("a table"),
        );
        let mut edits = Edits::default();
        edits.set(field("agents.model", json!("tiny")), None);
        assert_eq!(
            serde_json::Value::Object(pane.edited_table_with(&edits)),
            json!({
                "name": "x",
                "agents": { "model": "tiny" },
                "budget": { "model": "large", "cap": 3 },
            })
        );
    }

    #[test]
    fn a_dog_schema_pane_has_no_groups() {
        let pane = tree_pane();
        assert!(
            pane.fields()
                .fields()
                .iter()
                .all(|field| field.group.is_none())
        );
    }
}
