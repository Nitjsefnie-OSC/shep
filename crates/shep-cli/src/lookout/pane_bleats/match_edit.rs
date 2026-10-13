//! The match box: the text being typed, with its cursor, over the match axis
//! it narrows live.

use super::{BleatsPane, TextBox};

/// The match box while it is open: the axis as it stood before the edit, and
/// the text being typed, which owns the cursor. The compiled matcher holds
/// the text but not where the cursor is in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MatchEdit {
    before: Option<String>,
    text: TextBox,
}

impl BleatsPane {
    /// Opens the match box, remembering the match axis's current text so
    /// [`Self::abandon_match_edit`] can restore it.
    pub fn begin_match_edit(&mut self) {
        let before = self.filters.match_text().map(str::to_string);
        let text = TextBox::from(before.as_deref().unwrap_or_default());
        self.match_snapshot = Some(MatchEdit { before, text });
    }

    /// Applies `edit` to the open match box, then narrows the match axis to
    /// what the box now holds. Nothing happens while the box is closed.
    pub fn edit_match_box(&mut self, edit: impl FnOnce(&mut TextBox)) {
        let Some(open) = self.match_snapshot.as_mut() else {
            return;
        };
        edit(&mut open.text);
        let text = open.text.as_str().to_string();
        self.set_match(text);
    }

    /// The match box's live buffer while it is open, or `None` when it is
    /// not.
    ///
    /// The status bar needs both facts and the buffer alone cannot carry
    /// them: an empty box and a closed box both read as an empty matcher.
    /// `Some` of an empty box is a box open over nothing typed yet.
    #[must_use]
    pub fn match_editing(&self) -> Option<&TextBox> {
        self.match_snapshot.as_ref().map(|open| &open.text)
    }

    /// Ends the match box, keeping whatever [`Self::set_match`] already
    /// applied on the way in: `TextChar` and `TextBackspace` narrow the
    /// axis live, so there is nothing left for this to write.
    pub fn commit_match_edit(&mut self) {
        self.match_snapshot = None;
    }

    /// Restores the match axis to what [`Self::begin_match_edit`] saw,
    /// discarding whatever was typed since. A no-op if the match box was
    /// never opened.
    pub fn abandon_match_edit(&mut self) {
        if let Some(open) = self.match_snapshot.take() {
            self.set_match(open.before.unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::lookout::textbox::TextEdit;

    /// Abandoning an edit restores what the axis held before it, compiled:
    /// the regex that was there has to work again, not merely have its text
    /// back.
    #[test]
    fn abandoning_an_edit_restores_the_earlier_matcher_compiled() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.set_match("/po+l/".to_string());

        pane.begin_match_edit();
        pane.set_match("kennel".to_string());
        assert_eq!(pane.filters().match_kind(), Some(MatchKind::Literal));

        pane.abandon_match_edit();
        assert_eq!(pane.filters().match_kind(), Some(MatchKind::Regex));
        assert_eq!(pane.filters().match_text(), Some("/po+l/"));
        assert_eq!(
            pane.filters().match_ranges("pol and pool"),
            vec![(0, 3), (8, 12)],
            "the restored regex matches, rather than being a dead string"
        );
    }

    /// The mirror, and the one a stale matcher survives: abandoning back to
    /// a literal must not leave the regex typed during the edit compiled and
    /// still matching.
    #[test]
    fn abandoning_an_edit_back_to_a_literal_drops_the_regex_typed_during_it() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.set_match("po+l".to_string());

        pane.begin_match_edit();
        pane.set_match("/po+l/".to_string());
        pane.abandon_match_edit();

        assert_eq!(pane.filters().match_kind(), Some(MatchKind::Literal));
        assert!(
            pane.filters().match_ranges("pool").is_empty(),
            "the abandoned regex must not still be matching"
        );
        assert_eq!(pane.filters().match_ranges("po+l"), vec![(0, 4)]);
    }

    /// Abandoning an edit that began with no axis set clears it again,
    /// rather than keeping whatever was typed before the escape.
    #[test]
    fn abandoning_an_edit_that_began_unset_leaves_the_axis_unset() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.begin_match_edit();
        pane.set_match("/po+l/".to_string());
        pane.abandon_match_edit();

        assert_eq!(pane.filters().match_text(), None);
        assert!(pane.filters().is_empty());
    }

    /// Abandon with no edit open is a no-op: the axis keeps what it has.
    #[test]
    fn abandoning_with_no_edit_open_leaves_the_axis_alone() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.set_match("/po+l/".to_string());
        pane.abandon_match_edit();

        assert_eq!(pane.filters().match_kind(), Some(MatchKind::Regex));
        assert_eq!(pane.filters().match_ranges("pool"), vec![(0, 4)]);
    }

    /// Committing keeps what was typed during the edit, compiled as its own
    /// text rather than as the text the edit started from.
    #[test]
    fn committing_an_edit_keeps_the_matcher_typed_during_it() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.set_match("pool".to_string());

        pane.begin_match_edit();
        pane.set_match("/po+l/".to_string());
        pane.commit_match_edit();

        assert_eq!(pane.filters().match_kind(), Some(MatchKind::Regex));
        assert_eq!(pane.filters().match_ranges("pol"), vec![(0, 3)]);
        assert_eq!(pane.match_editing(), None, "the box is closed");
    }

    /// The match box keeps its own cursor, and the axis follows its text.
    #[test]
    fn editing_the_match_box_in_the_middle_narrows_the_axis_to_the_whole_text() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.begin_match_edit();
        for typed in "pol".chars() {
            pane.edit_match_box(|text| text.insert(typed));
        }
        pane.edit_match_box(|text| text.edit(TextEdit::Left));
        pane.edit_match_box(|text| text.insert('o'));

        assert_eq!(pane.match_editing().map(TextBox::as_str), Some("pool"));
        assert_eq!(pane.match_editing().map(TextBox::cursor), Some(3));
        assert_eq!(pane.filters().match_text(), Some("pool"));
    }

    /// Abandoning restores the text from before the edit, cursor and all.
    #[test]
    fn abandoning_after_a_cursor_edit_restores_the_axis() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.set_match("old".to_string());
        pane.begin_match_edit();
        assert_eq!(pane.match_editing().map(TextBox::cursor), Some(3));
        pane.edit_match_box(|text| text.edit(TextEdit::DeleteToStart));
        assert_eq!(pane.filters().match_text(), None, "emptied, so cleared");

        pane.abandon_match_edit();

        assert_eq!(pane.filters().match_text(), Some("old"));
        assert_eq!(pane.match_editing(), None);
    }

    /// With the box closed an edit is dropped rather than changing the axis.
    #[test]
    fn a_match_edit_with_the_box_closed_changes_nothing() {
        let mut pane = BleatsPane::new(RowKey::Sheep(9));
        pane.set_match("old".to_string());

        pane.edit_match_box(|text| text.insert('x'));

        assert_eq!(pane.filters().match_text(), Some("old"));
    }
}
