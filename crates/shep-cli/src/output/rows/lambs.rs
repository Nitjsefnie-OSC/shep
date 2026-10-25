//! A sheep's lamb tree as `describe`'s second table: [`LambRows`], or
//! [`LabelledLambRows`] once the sheep has named any of its lambs.

use serde::Serialize;
use shep_core::protocol::Lamb;

use crate::output::Render;
use crate::style::Presentation;
use crate::terminal_safe::sanitise;

use super::toolkit::{Paint, paint};

/// One sheep's lamb tree, as `describe`'s second table.
///
/// Not `#[serde(transparent)]`: this type's JSON is never read, since
/// `describe --format json` serializes the listing as
/// [`FlockRows`](super::FlockRows) with its own `lambs`. It exists to reach
/// [`render_table`](crate::output::render_table).
#[derive(Debug, Serialize)]
pub struct LambRows(pub Vec<Lamb>);

/// No colour: both columns are identity, and a lamb has no status, reading or
/// placeholder for one to carry.
impl Render for LambRows {
    fn headers() -> &'static [&'static str] {
        &["PID", "NAME"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|lamb| vec![lamb.pid.to_string(), lamb.name.clone()])
            .collect()
    }

    // Parallel to `headers()`. Two columns, both identity, so this never
    // narrows; spelled out so a later header does not inherit it by omission.
    const PRIORITIES: &'static [u8] = &[0, 0];
}

/// [`LambRows`] plus a LABEL column, for a tree where the sheep named at
/// least one lamb on its shepherd channel.
///
/// Its own table so a tree nobody labelled renders as it always has. JSON
/// is never read, for [`LambRows`]'s reason.
#[derive(Debug, Serialize)]
pub struct LabelledLambRows(pub Vec<Lamb>);

impl LabelledLambRows {
    /// Whether `lambs` needs this table rather than [`LambRows`].
    pub fn wanted_for(lambs: &[Lamb]) -> bool {
        lambs.iter().any(|lamb| lamb.label.is_some())
    }
}

/// Only the `-` of an unlabelled lamb is painted: PID and NAME are identity,
/// as in [`LambRows`].
impl Render for LabelledLambRows {
    fn headers() -> &'static [&'static str] {
        &["PID", "NAME", "LABEL"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|lamb| {
                vec![
                    lamb.pid.to_string(),
                    lamb.name.clone(),
                    // The app worded it, so a bidi override goes too.
                    lamb.label
                        .as_deref()
                        .map_or_else(|| "-".to_string(), |label| sanitise(label).0),
                ]
            })
            .collect()
    }

    fn rows_for(&self, presentation: Presentation, status_word: bool) -> Vec<Vec<String>> {
        paint(
            self.rows(),
            Self::headers(),
            presentation,
            status_word,
            |_, _, _| Paint::Default,
        )
    }

    // Parallel to `headers()`. The label is the app's own text, so it goes
    // first; PID and NAME are identity.
    const PRIORITIES: &'static [u8] = &[0, 0, 6];
}

#[cfg(test)]
mod tests {
    use crate::vocabulary::Role;

    use super::super::tests::{assert_no_drift, coloured, painted};
    use super::*;

    #[test]
    fn lamb_rows_do_not_drift() {
        assert_no_drift(
            &LambRows(vec![Lamb::new(4243, "node"), Lamb::new(4244, "sh")]),
            |j| &j[0],
            &[],
        );
    }

    #[test]
    fn lamb_rows_carry_no_colour_at_all() {
        let rows = LambRows(vec![Lamb::new(48_302, "node")]).rows_for(coloured(), true);
        assert_eq!(rows[0], vec!["48302".to_string(), "node".to_string()]);
    }

    #[test]
    fn labelled_lamb_rows_do_not_drift() {
        assert_no_drift(
            &LabelledLambRows(vec![
                Lamb::new(4243, "python").with_label("worker 1"),
                Lamb::new(4244, "sh"),
            ]),
            |j| &j[0],
            &[],
        );
    }

    #[test]
    fn only_a_missing_label_is_painted() {
        let rows = LabelledLambRows(vec![
            Lamb::new(48_302, "python").with_label("worker 1"),
            Lamb::new(48_303, "sh"),
        ])
        .rows_for(coloured(), true);
        assert_eq!(
            rows,
            vec![
                vec![
                    "48302".to_string(),
                    "python".to_string(),
                    "worker 1".to_string()
                ],
                vec![
                    "48303".to_string(),
                    "sh".to_string(),
                    painted("-", Role::Ink3)
                ],
            ]
        );
    }

    /// `Lamb::label` is a plain `String`, not a `LambLabel`, past the
    /// daemon's own construction of it: this table is the last place that
    /// can catch a bidi override before it reaches a terminal.
    #[test]
    fn a_label_is_sanitised_before_it_reaches_the_cell() {
        let rows = LabelledLambRows(vec![
            Lamb::new(48_302, "python").with_label("worker\u{202e}1"),
        ])
        .rows();
        assert_eq!(rows[0][2], "worker1");
    }

    #[test]
    fn the_labelled_table_is_wanted_only_once_a_lamb_has_a_label() {
        assert!(!LabelledLambRows::wanted_for(&[]));
        assert!(!LabelledLambRows::wanted_for(&[Lamb::new(1, "sh")]));
        assert!(LabelledLambRows::wanted_for(&[
            Lamb::new(1, "sh"),
            Lamb::new(2, "sh").with_label("b"),
        ]));
    }
}
