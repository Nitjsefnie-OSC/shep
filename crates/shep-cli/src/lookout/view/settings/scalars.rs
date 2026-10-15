//! The settings screen's six scalar rows: which columns fit, and
//! what each cell says.
//!
//! NAME, VALUE, SOURCE and the apply cost, measured against a BODY
//! width ([`super::body_width`]) rather than the raw terminal.

use ratatui::text::{Line, Span};

use super::super::flock::{fit, mark};
use crate::commands::settings::{ScalarView, SettingField, SettingsSnapshot};
use crate::style::StyleSource;

/// Which of the six scalars a [`SettingField`] names.
pub(super) fn scalar_view(snapshot: &SettingsSnapshot, field: SettingField) -> &ScalarView {
    match field {
        SettingField::LogLevel => &snapshot.log_level,
        SettingField::LogJson => &snapshot.log_json,
        SettingField::Socket => &snapshot.socket,
        SettingField::MaxCronSleep => &snapshot.max_cron_sleep,
        SettingField::AllowControl => &snapshot.allow_control,
        SettingField::StyleLevel => &snapshot.style_level,
    }
}

/// The name printed in the NAME cell: the document's own key, except
/// `[style] level`, which drops the `style_` a section header already says.
///
/// `pub(in crate::lookout::view)`: `status::status_line` names the field
/// being typed with this same word, so the status bar and body pane agree.
pub(in crate::lookout::view) const fn field_label(field: SettingField) -> &'static str {
    field.key()
}

/// What applying this field costs. `log_level`, `log_json` and
/// `max_cron_sleep` need a daemon reload; `socket` needs a stop and start,
/// since reload never moves the listening socket; `allow_control` needs
/// whistle restarted, not the daemon.
///
/// `style level` reads `source` too: costs nothing when the file decides,
/// since lookout re-resolves it next command, but `--style` and
/// `$SHEP_STYLE` outrank the file and make that untrue.
const fn apply_cost(field: SettingField, source: StyleSource) -> &'static str {
    match field {
        SettingField::LogLevel | SettingField::LogJson | SettingField::MaxCronSleep => {
            "needs shep daemon reload"
        }
        SettingField::Socket => "needs the shepherd stopped and started",
        SettingField::AllowControl => "needs shep whistle restarted",
        SettingField::StyleLevel => match source {
            StyleSource::Config | StyleSource::Default => "the next command reads it",
            StyleSource::Env | StyleSource::Flag => "written, but outranked",
        },
    }
}

/// NAME column width: fits `max_cron_sleep`, the longest field name, with a
/// column of padding.
const SCALAR_NAME_W: u16 = 15;
/// VALUE column width where the terminal can afford it: fits
/// `/home/ada/.shep/run/shep.sock` (29 columns) whole. A cap, not a fixed
/// width: a narrow terminal gets [`SCALAR_VALUE_MIN`] instead, and anything
/// wider truncates through [`fit`].
const SCALAR_VALUE_W: u16 = 30;
/// The floor on VALUE: enough for `not set`, `waiting`, a level name or the
/// tail of a path, never a whole socket path. Below this the row stops
/// saying anything and there is nothing left to trade.
const SCALAR_VALUE_MIN: u16 = 10;
/// SOURCE column width: `$SHEP_STYLE` and `the default` are both 11 columns,
/// the widest two words [`crate::style::StyleSource`]'s `Display` ever
/// prints.
const SCALAR_SOURCE_W: u16 = 11;
/// The floor on the apply-cost column once the terminal is too narrow to
/// give it the remainder: enough for `needs`, never a whole sentence.
const SCALAR_COST_MIN: u16 = 8;

/// One column of a scalar row.
///
/// `Debug` is derived, not redacted: a bare variant name leaks nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarColumn {
    /// The document's own key for this field.
    Name,
    /// What the file says, or the compiled fallback when it says nothing.
    /// The flexible column, between [`SCALAR_VALUE_MIN`] and
    /// [`SCALAR_VALUE_W`].
    Value,
    /// Which layer the value came from.
    Source,
    /// What applying an edit to this field costs. Takes whatever the other
    /// three leave.
    Cost,
}

const SCALAR_ALL: &[ScalarColumn] = &[
    ScalarColumn::Name,
    ScalarColumn::Value,
    ScalarColumn::Source,
    ScalarColumn::Cost,
];
const SCALAR_NO_COST: &[ScalarColumn] = &[
    ScalarColumn::Name,
    ScalarColumn::Value,
    ScalarColumn::Source,
];
const SCALAR_FLOOR: &[ScalarColumn] = &[ScalarColumn::Name, ScalarColumn::Value];

/// The narrowest the scalar rows will draw into, as a body width: NAME, a
/// gap and VALUE at its floor. The terminal also pays
/// [`super::super::flock::GUTTER`], the same split
/// [`super::dogs::DOG_MIN_WIDTH`] documents.
const SCALAR_MIN_WIDTH: u16 = SCALAR_NAME_W + 2 + SCALAR_VALUE_MIN;

/// Width thresholds for the scalar rows, widest first, the same shape
/// [`super::dogs::DOG_TIERS`] and `flock::columns::TIERS` both use.
///
/// The apply cost drops first, then SOURCE, the reverse of the dogs
/// table's order: the cost also shows in the confirm and the status bar,
/// while SOURCE appears nowhere else. NAME and VALUE are the floor.
const SCALAR_TIERS: &[(u16, &[ScalarColumn])] = &[
    (
        SCALAR_NAME_W + 2 + SCALAR_VALUE_MIN + 2 + SCALAR_SOURCE_W + 2 + SCALAR_COST_MIN,
        SCALAR_ALL,
    ),
    (
        SCALAR_NAME_W + 2 + SCALAR_VALUE_MIN + 2 + SCALAR_SOURCE_W,
        SCALAR_NO_COST,
    ),
    (SCALAR_MIN_WIDTH, SCALAR_FLOOR),
];

/// The widest scalar column set that fits `width`, a BODY width.
fn scalar_columns_for(width: u16) -> &'static [ScalarColumn] {
    SCALAR_TIERS
        .iter()
        .find(|(threshold, _)| width >= *threshold)
        .map_or(SCALAR_FLOOR, |(_, columns)| *columns)
}

/// What VALUE and the apply cost get out of `width`, once NAME, SOURCE and
/// the separators are paid for.
///
/// VALUE takes the remainder up to [`SCALAR_VALUE_W`] and never below
/// [`SCALAR_VALUE_MIN`]; the cost takes what is left after that, so a wide
/// terminal spends its extra columns on the sentence rather than on padding
/// a value that has already been shown whole. The cost's number is zero
/// when [`ScalarColumn::Cost`] is not in `columns`, and nothing reads it
/// then.
fn scalar_widths(width: u16, columns: &[ScalarColumn]) -> (u16, u16) {
    let fixed = SCALAR_NAME_W
        + if columns.contains(&ScalarColumn::Source) {
            SCALAR_SOURCE_W
        } else {
            0
        };
    let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(0) * 2;
    let remainder = width.saturating_sub(fixed).saturating_sub(gaps);
    if columns.contains(&ScalarColumn::Cost) {
        let value = remainder
            .saturating_sub(SCALAR_COST_MIN)
            .clamp(SCALAR_VALUE_MIN, SCALAR_VALUE_W);
        (value, remainder.saturating_sub(value).max(SCALAR_COST_MIN))
    } else {
        (remainder.clamp(SCALAR_VALUE_MIN, SCALAR_VALUE_W), 0)
    }
}

/// One scalar cell's text.
fn scalar_cell(field: SettingField, view: &ScalarView, column: ScalarColumn) -> String {
    match column {
        ScalarColumn::Name => field_label(field).to_owned(),
        ScalarColumn::Value => view.value.clone(),
        ScalarColumn::Source => view.source.to_string(),
        ScalarColumn::Cost => apply_cost(field, view.source).to_string(),
    }
}

/// One scalar row: name, value, source, apply cost, as many of those as
/// `width` (a BODY width) can pay for.
pub(super) fn scalar_line(
    field: SettingField,
    view: &ScalarView,
    selected: bool,
    width: u16,
) -> Line<'static> {
    let columns = scalar_columns_for(width);
    let (value_width, cost_width) = scalar_widths(width, columns);
    let mut text = format!("{} ", mark(selected));
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            text.push_str("  ");
        }
        let cell_width = match column {
            ScalarColumn::Name => SCALAR_NAME_W,
            ScalarColumn::Value => value_width,
            ScalarColumn::Source => SCALAR_SOURCE_W,
            ScalarColumn::Cost => cost_width,
        };
        text.push_str(&fit(&scalar_cell(field, view, *column), cell_width));
    }
    Line::from(Span::raw(text))
}

#[cfg(test)]
mod tests {
    use super::super::super::fixtures;
    use super::super::content_lines;
    use super::*;
    use crate::style::StyleSource;

    /// Widths here are BODY widths,
    /// [`super::super::super::flock::GUTTER`] columns narrower than the
    /// terminal.
    #[test]
    fn the_scalar_apply_cost_drops_before_its_source() {
        assert!(scalar_columns_for(118).contains(&ScalarColumn::Cost));
        assert!(!scalar_columns_for(43).contains(&ScalarColumn::Cost));
        assert!(
            scalar_columns_for(43).contains(&ScalarColumn::Source),
            "SOURCE is the screen's own subject and outlives the cost"
        );
        assert!(!scalar_columns_for(33).contains(&ScalarColumn::Source));
        assert!(
            scalar_columns_for(33).contains(&ScalarColumn::Value),
            "NAME and VALUE are the floor"
        );
    }

    /// No fixture in this file renders `Default`: `settings_snapshot` gives
    /// every scalar `StyleSource::Config`, one column shorter.
    #[test]
    fn the_default_source_label_fits_the_column_it_was_sized_for() {
        let rendered = fit(&StyleSource::Default.to_string(), SCALAR_SOURCE_W);
        assert_eq!(
            rendered.chars().count(),
            usize::from(SCALAR_SOURCE_W),
            "fit always pads or truncates to the exact column width"
        );
        assert!(
            !rendered.contains('…'),
            "SCALAR_SOURCE_W must fit \"the default\" whole: got {rendered:?}"
        );
    }

    #[test]
    fn the_style_cost_cell_stops_promising_the_next_command_when_it_is_outranked() {
        let app = fixtures::app_in_settings_with_shadowed_style(StyleSource::Env);
        let settings = app.settings().unwrap();
        let lines = content_lines(&app, settings, app.palette(), 120, 0);
        let rendered: Vec<String> = lines
            .iter()
            .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let row = rendered
            .iter()
            .find(|line| {
                line.get(2..)
                    .is_some_and(|cells| cells.starts_with("level "))
            })
            .unwrap_or_else(|| panic!("the style row is drawn: {rendered:?}"));
        assert!(row.contains("$SHEP_STYLE"), "got: {row:?}");
        assert!(row.contains("written, but outranked"), "got: {row:?}");
        assert!(!row.contains("the next command reads it"), "got: {row:?}");
    }
}
