//! The dogs table on the settings screen: which columns fit, and
//! what each row's cells say.
//!
//! A row joins the file's own `[daemon] enabled_dogs` entry against
//! the live flock by name. `IN FILE` answers the file and `RUNNING`
//! answers the flock, and neither is a claim about the other.

use std::path::PathBuf;

use ratatui::text::{Line, Span};

use super::super::super::app::App;
use super::super::super::theme::Palette;
use super::super::flock::{fit, mark};
use crate::vocabulary::Reported;

/// The dogs caption, verbatim: arms and confirms like every other lookout
/// action (`x`, `R`, `L`).
pub(super) const DOGS_CAPTION: &str = "space arms, Enter applies; a dog needs no reload";

/// The floor on the dogs table's NAME column, mirroring
/// [`super::super::flock::columns::NAME_MIN`]: never shrinks below a name worth
/// reading.
const DOG_NAME_MIN: u16 = 8;

/// One column of the dogs table.
///
/// `Debug` is derived, not redacted: a bare variant name leaks nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DogColumn {
    /// The dog's name. The flexible column.
    Name,
    /// Whether `[daemon] enabled_dogs` names it, not whether the shepherd
    /// has started it.
    InFile,
    /// `built-in`, or the adopted binary's path.
    Source,
    /// Whether the dog is up: [`dog_rows`] joins it against the live
    /// flock, reading `not running` when no dog by that name is running.
    Running,
}

impl DogColumn {
    /// The header text.
    #[must_use]
    const fn header(self) -> &'static str {
        match self {
            Self::Name => "NAME",
            Self::InFile => "IN FILE",
            Self::Source => "SOURCE",
            Self::Running => "RUNNING",
        }
    }

    /// The fixed width of this column's cells. `Name` reports `0`; see
    /// [`super::super::flock::columns::Column::width`].
    #[must_use]
    const fn width(self) -> u16 {
        match self {
            Self::Name => 0,
            // `IN FILE` header; yes/no cells are shorter.
            Self::InFile => 7,
            // budget for an adopted binary's path before `fit` truncates.
            Self::Source => 24,
            // matches `super::super::flock::Column::Status`'s width.
            Self::Running => 15,
        }
    }
}

const ALL_DOG_COLUMNS: &[DogColumn] = &[
    DogColumn::Name,
    DogColumn::InFile,
    DogColumn::Source,
    DogColumn::Running,
];
const NO_SOURCE: &[DogColumn] = &[DogColumn::Name, DogColumn::InFile, DogColumn::Running];
const FLOOR_DOG_COLUMNS: &[DogColumn] = &[DogColumn::Name, DogColumn::Running];

/// The narrowest the dogs table will draw into: [`DogColumn::Running`] plus
/// [`DOG_NAME_MIN`] plus one gap.
///
/// A table width, not a terminal width, like
/// [`super::super::flock::MIN_WIDTH`]: the narrowest terminal this table
/// fits is `DOG_MIN_WIDTH + GUTTER`. [`DOG_TIERS`]'s thresholds compare
/// against [`super::body_width`], never the raw terminal.
pub(super) const DOG_MIN_WIDTH: u16 = DogColumn::Running.width() + DOG_NAME_MIN + 2;

/// Width thresholds, widest first, mirroring `flock::columns::TIERS`:
/// least-diagnostic column drops first. SOURCE (widest) drops before IN
/// FILE; NAME and RUNNING are the floor.
pub(super) const DOG_TIERS: &[(u16, &[DogColumn])] = &[
    (61, ALL_DOG_COLUMNS),
    (34, NO_SOURCE),
    (DOG_MIN_WIDTH, FLOOR_DOG_COLUMNS),
];

/// The widest dogs-table column set that fits `width`.
///
/// Includes [`DogColumn::Running`] in every tier, alongside NAME: it is the
/// floor, and `draw_settings` renders every column this returns.
#[must_use]
pub fn columns_for(width: u16) -> &'static [DogColumn] {
    DOG_TIERS
        .iter()
        .find(|(threshold, _)| width >= *threshold)
        .map_or(FLOOR_DOG_COLUMNS, |(_, columns)| *columns)
}

/// What NAME gets once the fixed dogs-table columns and their separators
/// are paid for. [`super::super::flock::columns::name_width`]'s own twin.
fn dog_name_width(width: u16, columns: &[DogColumn]) -> u16 {
    let fixed: u16 = columns.iter().map(|column| column.width()).sum();
    let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(0) * 2;
    width
        .saturating_sub(fixed)
        .saturating_sub(gaps)
        .max(DOG_NAME_MIN)
}

/// One row of the dogs table: the file joined against the live flock, by
/// name. [`DogView`](crate::commands::settings::DogView) supplies `enabled`
/// and `adopted_path`; [`dog_rows`] adds `running`.
///
/// `Debug` is derived, not redacted: name, bool, word and path, none of it
/// a secret. A dog's own config lives in `dogs.toml`, never here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DogRow {
    /// The dog's name.
    pub name: String,
    /// Whether `[daemon] enabled_dogs` names it, not whether the shepherd
    /// has started it.
    pub enabled: bool,
    /// The word the flock table would show for this dog's own row, or
    /// `None` when no dog of this name is running.
    pub running: Option<String>,
    /// `None` for a built-in dog; the adopted binary's path otherwise.
    pub adopted_path: Option<PathBuf>,
}

/// The dogs table's rows: `app`'s settings snapshot joined against its own
/// live flock, by name.
///
/// Takes `app` rather than `Settings` alone: the file half
/// ([`crate::commands::settings::SettingsSnapshot::dogs`]) and the live
/// flock (`App::all_rows`) live on two different types. Returns an empty
/// vector when the settings screen is not open.
#[must_use]
pub fn dog_rows(app: &App, width: u16) -> Vec<DogRow> {
    // unused: every dogs-table tier keeps `DogColumn::Running`, so no width
    // leaves the join unrendered. Kept for signature parity with the other
    // view functions.
    let _ = width;
    let Some(settings) = app.settings() else {
        return Vec::new();
    };
    let running_by_name: std::collections::BTreeMap<&str, String> = app
        .all_rows()
        .into_iter()
        .filter(|row| row.info.dog.is_some())
        .map(|row| {
            (
                row.info.name.as_str(),
                Reported::of(row.info.status, row.info.handshook).word(),
            )
        })
        .collect();
    settings
        .snapshot()
        .dogs
        .iter()
        .map(|dog| DogRow {
            name: dog.name.clone(),
            enabled: dog.enabled,
            running: running_by_name.get(dog.name.as_str()).cloned(),
            adopted_path: dog.adopted_path.clone(),
        })
        .collect()
}

/// One dog's cell text.
fn dog_cell(dog: &DogRow, column: DogColumn) -> String {
    match column {
        DogColumn::Name => dog.name.clone(),
        DogColumn::InFile => if dog.enabled { "yes" } else { "no" }.to_string(),
        DogColumn::Source => dog
            .adopted_path
            .as_ref()
            .map_or_else(|| "built-in".to_string(), |path| path.display().to_string()),
        DogColumn::Running => dog
            .running
            .clone()
            .unwrap_or_else(|| "not running".to_string()),
    }
}

/// The dogs table's header line, indented to match every row's own
/// mark-and-gap prefix ([`super::super::flock::mark`]'s own two columns).
pub(super) fn dog_header_line(
    columns: &[DogColumn],
    width: u16,
    palette: Palette,
) -> Line<'static> {
    let name_width = dog_name_width(width, columns);
    let mut text = String::from("  ");
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            text.push_str("  ");
        }
        let cell_width = if *column == DogColumn::Name {
            name_width
        } else {
            column.width()
        };
        text.push_str(&fit(column.header(), cell_width));
    }
    Line::from(Span::styled(text, palette.muted()))
}

/// One dog's row.
pub(super) fn dog_line(
    dog: &DogRow,
    columns: &[DogColumn],
    width: u16,
    selected: bool,
) -> Line<'static> {
    let name_width = dog_name_width(width, columns);
    let mut text = format!("{} ", mark(selected));
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            text.push_str("  ");
        }
        let cell_width = if *column == DogColumn::Name {
            name_width
        } else {
            column.width()
        };
        text.push_str(&fit(&dog_cell(dog, *column), cell_width));
    }
    Line::from(Span::raw(text))
}

#[cfg(test)]
mod tests {
    use super::super::super::fixtures;
    use super::super::super::flock::GUTTER;
    use super::super::content_lines;
    use super::*;
    use crate::output::width::visible_width;

    /// SOURCE is the widest column, so it drops first.
    #[test]
    fn the_dogs_source_column_drops_before_the_rest() {
        assert!(columns_for(120).contains(&DogColumn::Source));
        assert!(!columns_for(60).contains(&DogColumn::Source));
        assert!(
            columns_for(60).contains(&DogColumn::Running),
            "RUNNING is the diagnostic half and outlives SOURCE"
        );
    }

    /// Measures the rendered lines, not the declared widths: each line
    /// carries `mark`'s own two-column prefix, which a width sum would miss.
    #[test]
    fn every_dogs_tier_fits_the_width_it_claims() {
        let app = fixtures::app_in_settings_with_dog_drift();
        let settings = app.settings().unwrap();
        let palette = app.palette();
        for width in (DOG_MIN_WIDTH + GUTTER)..=200 {
            let rendered: Vec<String> = content_lines(&app, settings, palette, width, 0)
                .iter()
                .map(|line| line.spans.iter().map(|s| s.content.as_ref()).collect())
                .collect();
            // Everything from the `[dogs]` header down: the caption, the
            // column header and one line per dog.
            let table = rendered
                .iter()
                .position(|line| line.contains("[dogs]"))
                .expect("the dogs section is always drawn");
            for line in &rendered[table..] {
                assert!(
                    visible_width(line) <= usize::from(width),
                    "width {width} drew {} columns: {line:?}",
                    visible_width(line)
                );
            }
        }
    }

    #[test]
    fn the_dogs_caption_says_arms_not_applies() {
        assert_eq!(
            DOGS_CAPTION,
            "space arms, Enter applies; a dog needs no reload"
        );
    }

    /// `otel` runs while the file has it disabled, which is what "a removed
    /// name keeps running" looks like. `ledger` is enabled and absent,
    /// which is a dog that failed to start.
    #[test]
    fn the_dogs_table_joins_the_file_against_the_running_flock() {
        let app = fixtures::app_in_settings_with_dog_drift();
        let rows = dog_rows(&app, 120);

        let otel = rows.iter().find(|r| r.name == "otel").unwrap();
        assert!(!otel.enabled);
        assert_eq!(otel.running.as_deref(), Some("online"));

        let ledger = rows.iter().find(|r| r.name == "ledger").unwrap();
        assert!(ledger.enabled);
        assert_eq!(ledger.running, None);
    }

    /// A dog that never completed a handshake reads `silent`, not `online`.
    #[test]
    fn a_dog_that_never_handshook_reads_silent_here_too() {
        let app = fixtures::app_in_settings_with_silent_dog();
        let rows = dog_rows(&app, 120);
        let bark = rows.iter().find(|r| r.name == "bark").unwrap();
        assert_eq!(bark.running.as_deref(), Some("silent"));
    }
}
