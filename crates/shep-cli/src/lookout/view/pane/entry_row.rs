//! A row under an array or map of tables: an entry, a table in one, or one
//! of its fields. Also a name in a checklist.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::super::super::pane::{ConfigPane, EntryBody, PaneRow};
use super::super::super::theme::Palette;
use super::super::flock::{fit, mark};
use super::field_row::{TABLE_LABEL, leaf_line, node_line};
use super::layout::{body_width, widths};

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

/// Name `item` of the checklist under field `field`, drawn as its
/// `[x] name` text, indented to its depth. It is editable, so it is not muted.
pub(super) fn choice_line(
    pane: &ConfigPane,
    row: PaneRow,
    selected: bool,
    width: u16,
    palette: Palette,
) -> Line<'static> {
    let Some(tree) = pane.tree_row(row) else {
        return Line::default();
    };
    let (key_w, _, _) = widths(body_width(width), false);
    let ground = if selected {
        palette.ground()
    } else {
        Style::default()
    };
    let mark_style = if selected {
        palette.attention().patch(ground)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::styled(mark(selected), mark_style),
        Span::styled(
            format!(
                " {}",
                fit(
                    &format!(" {}{}", "  ".repeat(tree.depth), tree.label),
                    key_w
                )
            ),
            ground,
        ),
    ])
}
