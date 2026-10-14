//! The payload `shep copy` reports: one row per key the copy touches, and
//! the run's own verdict beside it.

use core::fmt;

use serde::Serialize;

use crate::output::Render;
use crate::style::Presentation;
use crate::vocabulary::Role;

use super::toolkit::{Paint, paint};

/// One key a copy touches, and the two values it sits between.
///
/// `Debug` is written out, not derived (IR-41): a dog's table can hold a
/// credential the way a sheep's `env` does, and `DogTable` already withholds
/// its values from a `{:?}`. The rendered table and envelope print them
/// because the operator asked for this copy; a log line never does.
#[derive(Clone, Serialize)]
pub struct CopyChange {
    /// The key, dotted, relative to the path the copy names.
    pub key: String,
    /// `added`, `changed`, or `removed`.
    pub change: &'static str,
    /// The value the target held, `None` for an added key.
    pub old: Option<serde_json::Value>,
    /// The value the target holds afterwards, `None` for a removed key.
    pub new: Option<serde_json::Value>,
}

impl fmt::Debug for CopyChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let slot = |value: &Option<serde_json::Value>| {
            if value.is_some() { "<value>" } else { "none" }
        };
        write!(
            f,
            "CopyChange {{ key: {:?}, change: {:?}, old: {}, new: {} }}",
            self.key,
            self.change,
            slot(&self.old),
            slot(&self.new),
        )
    }
}

/// One value as a table cell: a string bare, anything else as compact JSON.
///
/// The JSON form carries the [`serde_json::Value`] itself, so a script never
/// has to guess whether `"2"` was a string or a number.
pub(crate) fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "<value>".to_string()),
    }
}

/// `shep copy`'s whole answer.
///
/// The four fields beside `changes` have no column: the table prints the
/// change list, which is the half an operator reads before it answers the
/// confirmation question, and the run's own state rides as a notice line.
/// `Debug` is derived: `changes` prints through [`CopyChange`]'s redacting
/// impl, and the rest names sheep, a path and two flags.
#[derive(Debug, Clone, Serialize)]
pub struct CopyReport {
    /// The sheep the values came from.
    pub from: String,
    /// The sheep they went to.
    pub to: String,
    /// The path copied, the same on both sides.
    pub path: String,
    /// Whether this run wrote anything.
    pub written: bool,
    /// Whether the operator asked for the changes and nothing else.
    pub dry_run: bool,
    /// Keys the target picked up only at its next spawn, naming the verb
    /// that promotes them.
    pub pending: Vec<String>,
    /// One entry per key the copy touches, in dotted-key order.
    pub changes: Vec<CopyChange>,
}

impl Render for CopyReport {
    fn headers() -> &'static [&'static str] {
        &["KEY", "CHANGE", "OLD", "NEW"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        self.changes
            .iter()
            .map(|change| {
                vec![
                    change.key.clone(),
                    change.change.to_string(),
                    change
                        .old
                        .as_ref()
                        .map_or_else(|| "-".to_string(), render_value),
                    change
                        .new
                        .as_ref()
                        .map_or_else(|| "-".to_string(), render_value),
                ]
            })
            .collect()
    }

    /// CHANGE alone: `removed` is the row that costs the operator something,
    /// so it takes `Bark` the way a failed installer step does.
    fn rows_for(&self, presentation: Presentation, status_word: bool) -> Vec<Vec<String>> {
        let rows = self.rows();
        paint(
            rows,
            Self::headers(),
            presentation,
            status_word,
            |header, cell, _index| match (header, cell) {
                ("CHANGE", "removed") => Paint::Role(Role::Bark),
                ("CHANGE", "added") => Paint::Role(Role::Meadow),
                _ => Paint::Default,
            },
        )
    }

    /// # Panics
    /// If `header` is not one of `Self::headers()`'s own values.
    #[track_caller]
    fn json_key_for(header: &str) -> &'static str {
        match header {
            "KEY" => "key",
            "CHANGE" => "change",
            "OLD" => "old",
            "NEW" => "new",
            other => panic!("CopyReport::headers() does not include {other:?}"),
        }
    }

    // Every field is a column of the change record; the run's own state has
    // no column for `rows`' reason.
    const JSON_ONLY: &'static [&'static str] = &[];

    // Parallel to `headers()`. KEY and CHANGE identify the row; the two
    // value columns are the unbounded ones a narrow terminal loses first.
    const PRIORITIES: &'static [u8] = &[0, 0, 6, 7];
}

#[cfg(test)]
mod tests {
    use super::super::tests::assert_no_drift;
    use super::*;

    fn report() -> CopyReport {
        CopyReport {
            from: "shep".to_string(),
            to: "shep-kelpie".to_string(),
            path: "dogs.kelpie.rulings".to_string(),
            written: true,
            dry_run: false,
            pending: Vec::new(),
            changes: vec![
                CopyChange {
                    key: "panel".to_string(),
                    change: "changed",
                    old: Some(serde_json::Value::String("issue".to_string())),
                    new: Some(serde_json::Value::String("panel".to_string())),
                },
                CopyChange {
                    key: "git.checkout".to_string(),
                    change: "added",
                    old: None,
                    new: Some(serde_json::json!("./worktree")),
                },
                CopyChange {
                    key: "legacy".to_string(),
                    change: "removed",
                    old: Some(serde_json::json!(4)),
                    new: None,
                },
            ],
        }
    }

    #[test]
    fn a_copy_report_does_not_drift() {
        // OLD and NEW are cells rendered from the value, so the cell and the
        // JSON field are deliberately different renderings of one value.
        assert_no_drift(&report(), |j| &j["changes"][0], &["OLD", "NEW"]);
    }

    /// The change list is the record count: the table and the JSON must
    /// never disagree about how many keys a copy touches.
    #[test]
    fn a_copy_report_renders_one_row_per_change() {
        let report = report();
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["changes"].as_array().unwrap().len(), 3);
        assert_eq!(report.rows().len(), 3);
    }

    /// A dog's table can hold a credential, and this is the type a log line
    /// would print (IR-41).
    #[test]
    fn a_copy_changes_debug_never_prints_a_value() {
        let change = CopyChange {
            key: "token".to_string(),
            change: "changed",
            old: Some(serde_json::Value::String("hunter2".to_string())),
            new: Some(serde_json::Value::String("rotated".to_string())),
        };
        assert_eq!(
            format!("{change:?}"),
            r#"CopyChange { key: "token", change: "changed", old: <value>, new: <value> }"#
        );
        let absent = CopyChange {
            key: "token".to_string(),
            change: "added",
            old: None,
            new: Some(serde_json::json!("hunter2")),
        };
        assert_eq!(
            format!("{absent:?}"),
            r#"CopyChange { key: "token", change: "added", old: none, new: <value> }"#
        );
        assert!(
            !format!("{:?}", report()).contains("hunter2"),
            "a report prints its changes through this impl"
        );
    }

    #[test]
    fn a_value_renders_as_a_bare_string_or_as_compact_json() {
        assert_eq!(render_value(&serde_json::json!("panel")), "panel");
        assert_eq!(render_value(&serde_json::json!(true)), "true");
        assert_eq!(render_value(&serde_json::json!(null)), "null");
        assert_eq!(render_value(&serde_json::json!([1, 2])), "[1,2]");
        assert_eq!(render_value(&serde_json::json!({ "a": 1 })), r#"{"a":1}"#);
    }
}
