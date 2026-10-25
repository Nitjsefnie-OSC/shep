//! The bleats pane's match axis: `/` opening the box, what typing into it
//! narrows live, abandoning it restoring what the axis held, and `n`/`N`
//! stepping between matches.
//!
//! Tests only, because the code they drive is shared and lives elsewhere: `/`
//! is claimed by the dispatcher in [`super::super::feed_keys`] ahead of the
//! pane's keymap, and `n`/`N` are two arms of [`super::keys`] that step
//! through its `scroll_bleats_back`. The sheep pane's embedded feed runs the
//! same dispatcher, so carving either out of there would split a shared keymap
//! in two.

#[cfg(test)]
mod tests {
    use super::super::*;
    use crate::lookout::view::fixtures;

    /// `/` opens the match input rather than doing nothing, which is what it
    /// did when this pane first shipped.
    #[test]
    fn slash_opens_the_match_input_in_the_bleats_pane() {
        let mut app =
            fixtures::with_selection(ProcessInfo::builder(9, "web", ProcStatus::Online).build());
        let _ = app.update(Msg::Key(KeyPress::Bleats));
        let _ = app.update(Msg::Key(KeyPress::FilterStart));
        assert_eq!(app.mode(), InputMode::Text, "typing goes to the pane");
    }

    /// Typing into the match box narrows live, the same as the flock
    /// table's own `/` box: the operator sees the filter row react to every
    /// keystroke rather than only after `Enter`.
    #[test]
    fn typing_in_the_match_box_narrows_live() {
        let mut app =
            fixtures::with_selection(ProcessInfo::builder(9, "web", ProcStatus::Online).build());
        let _ = app.update(Msg::Key(KeyPress::Bleats));
        let _ = app.update(Msg::Key(KeyPress::FilterStart));
        let _ = app.update(Msg::Key(KeyPress::TextChar('p')));
        let _ = app.update(Msg::Key(KeyPress::TextChar('o')));
        assert_eq!(
            app.bleats_pane().expect("open").filters().match_text(),
            Some("po")
        );
        let _ = app.update(Msg::Key(KeyPress::TextApply));
        assert_eq!(app.mode(), InputMode::Normal);
        assert_eq!(
            app.bleats_pane().expect("open").filters().match_text(),
            Some("po"),
            "TextApply keeps what was already applied live"
        );
    }

    /// `TextAbandon` restores the match axis to what it held before the box
    /// opened, discarding whatever was typed since, rather than clearing it
    /// outright the way the flock table's own filter box does.
    #[test]
    fn abandoning_the_match_box_restores_the_axis_it_had_before() {
        let mut app =
            fixtures::with_selection(ProcessInfo::builder(9, "web", ProcStatus::Online).build());
        let _ = app.update(Msg::Key(KeyPress::Bleats));
        app.bleats_pane_mut()
            .expect("open")
            .set_match("pool".to_string());

        let _ = app.update(Msg::Key(KeyPress::FilterStart));
        let _ = app.update(Msg::Key(KeyPress::TextChar('x')));
        assert_eq!(
            app.bleats_pane().expect("open").filters().match_text(),
            Some("poolx"),
            "typing narrowed live"
        );
        let _ = app.update(Msg::Key(KeyPress::TextAbandon));
        assert_eq!(app.mode(), InputMode::Normal);
        assert_eq!(
            app.bleats_pane().expect("open").filters().match_text(),
            Some("pool"),
            "abandon restores what the axis held before the edit"
        );
    }

    #[test]
    fn n_and_shift_n_step_between_matches_in_opposite_directions() {
        let mut app = fixtures::bleats_pane_with_lines(40);
        app.note_body_rows(6);
        app.bleats_pane_mut_for_tests()
            .expect("open")
            .set_match("line".to_string());
        assert!(app.bleats_pane().expect("open").following());

        let _ = app.update(Msg::Key(KeyPress::MatchPrev));
        let back = app.bleats_pane().expect("open").scroll_offset();
        assert!(back > 0, "N steps toward older matches");
        assert!(
            !app.bleats_pane().expect("open").following(),
            "stepping back is backward movement, so the follow stops"
        );

        let _ = app.update(Msg::Key(KeyPress::MatchNext));
        assert!(
            app.bleats_pane().expect("open").scroll_offset() < back,
            "n steps the other way"
        );
    }

    #[test]
    fn n_without_a_matcher_does_nothing() {
        let mut app = fixtures::bleats_pane_with_lines(120);
        let before = app.bleats_pane().expect("open").scroll_offset();
        let _ = app.update(Msg::Key(KeyPress::MatchNext));
        assert_eq!(app.bleats_pane().expect("open").scroll_offset(), before);
        assert!(app.bleats_pane().expect("open").following());
    }
}
