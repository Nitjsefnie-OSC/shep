//! A bracketed paste into the config pane's editors.

use super::super::testing::*;
use super::super::*;
use crate::lookout::textbox::TextEdit;
use crate::lookout::view::fixtures;

/// A pasted newline arrives as text, not as `Enter`: the editor takes
/// the printable characters and stays open, and the pane with it.
#[test]
fn a_paste_into_the_config_editor_drops_its_newline_and_keeps_the_editor_open() {
    let mut app = fixtures::app_in_sheep_pane_with_control();
    pane_to(&mut app, "cwd");
    app.update(Msg::Key(KeyPress::Confirm));
    app.update(Msg::Key(KeyPress::TextEdit(TextEdit::DeleteToStart)));

    let effect = app.update(Msg::Paste(Pasted::new("ab\ncd".to_string())));

    assert_eq!(effect, Effect::None);
    let pane = app.config_pane().expect("the pane is still open");
    assert_eq!(
        pane.typing().expect("the editor is open").buffer.as_str(),
        "abcd"
    );
    assert_eq!(app.mode(), InputMode::Text);
    assert!(pane.edits().is_empty(), "nothing was applied by the paste");
}

/// A paste lands at the cursor, not at the end.
#[test]
fn a_paste_lands_where_the_cursor_is() {
    let mut app = fixtures::app_in_sheep_pane_with_control();
    pane_to(&mut app, "cwd");
    app.update(Msg::Key(KeyPress::Confirm));
    app.update(Msg::Key(KeyPress::TextEdit(TextEdit::DeleteToStart)));
    for typed in "ad".chars() {
        app.update(Msg::Key(KeyPress::TextChar(typed)));
    }
    app.update(Msg::Key(KeyPress::TextEdit(TextEdit::Left)));

    app.update(Msg::Paste(Pasted::new("b\r\nc".to_string())));

    let typing = app
        .config_pane()
        .unwrap()
        .typing()
        .expect("the editor is open");
    assert_eq!(typing.buffer.as_str(), "abcd");
    assert_eq!(typing.buffer.cursor(), 3);
}

/// With no box open a paste is nothing: its characters are not keys.
#[test]
fn a_paste_with_no_editor_open_does_nothing() {
    let mut app = fixtures::app_in_sheep_pane_with_control();
    pane_to(&mut app, "cwd");
    let effect = app.update(Msg::Paste(Pasted::new("e\ndd".to_string())));

    assert_eq!(effect, Effect::None);
    assert_eq!(app.mode(), InputMode::Normal);
    let pane = app.config_pane().expect("the pane is still open");
    assert!(pane.typing().is_none(), "no editor opened");
    assert!(pane.edits().is_empty());
}
