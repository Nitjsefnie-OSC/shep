//! A row under an array or map of tables: an entry, a table in one, or one
//! of its fields.

use ratatui::text::Line;

use super::super::super::pane::{ConfigPane, EntryBody, PaneRow};
use super::super::super::theme::Palette;
use super::field_row::{TABLE_LABEL, leaf_line, node_line};

/// The type label of an entry's row, beside [`TABLE_LABEL`].
pub(super) const ENTRY_LABEL: &str = "entry";

/// Entry row `index`. An entry and a table in one open and close like any
/// table. A field draws as any other field row does: locked fields in it read
/// `~` and muted, and a secret reads `<set>`.
pub(super) fn entry_line(
    pane: &ConfigPane,
    index: usize,
    selected: bool,
    width: u16,
    palette: Palette,
    show_lands: bool,
) -> Line<'static> {
    let (Some(row), Some(tree)) = (pane.entry_row(index), pane.tree_row(PaneRow::Entry(index)))
    else {
        return Line::default();
    };
    let EntryBody::Leaf { field, .. } = &row.body else {
        let label = match &row.body {
            EntryBody::Node { entry: true, .. } => ENTRY_LABEL,
            EntryBody::Node { .. } | EntryBody::Leaf { .. } => TABLE_LABEL,
        };
        return node_line(&tree, label, selected, width, palette, show_lands);
    };
    let key = format!("{}{}", "  ".repeat(tree.depth), tree.label);
    leaf_line(pane, field, &key, selected, width, palette, show_lands)
}
