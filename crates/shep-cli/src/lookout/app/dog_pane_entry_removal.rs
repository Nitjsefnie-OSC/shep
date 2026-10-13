//! `d` on an entry of a dog pane's array or map of tables, driven by key
//! presses: what it files, what `u` and `Esc` do with that, and where the
//! cursor goes.

use super::dog_pane_entries::{
    ALPHA, close_with, open, open_every_client, pane, refresh_with, select, type_into,
};
use super::*;
use crate::lookout::app::testing::*;
use crate::lookout::view::fixtures::entries_dog_section;

/// The label of the entry row under the cursor.
#[track_caller]
fn cursor_label(app: &App) -> String {
    let pane = pane(app);
    let Some(PaneRow::Entry(index)) = pane.cursor() else {
        panic!("the cursor is on an entry: {:?}", pane.cursor());
    };
    pane.entry_row(index).expect("the row exists").label.clone()
}

const BETA: &str = "[[clients]]\nid = \"b-2\"\nkey = \"hunter3\"\n\n";

#[test]
fn d_on_an_entry_files_its_removal_and_sends_nothing_until_esc() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    select(&mut app, "clients[1]");
    assert_eq!(app.update(Msg::Key(KeyPress::Remove)), Effect::None);
    assert_eq!(pane(&app).edits().len(), 1);
    assert_eq!(pane(&app).value("clients"), "3 entries");
    assert_eq!(
        pane(&app).edited_value("clients").as_deref(),
        Some("2 entries")
    );
    let written = close_with(&mut app, &entries_dog_section());
    assert_eq!(written, entries_dog_section().replace(BETA, ""));
}

#[test]
fn an_edit_to_entry_two_and_the_removal_of_entry_zero_write_the_edit_on_what_is_now_the_second() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    type_into(&mut app, "clients[2].name", "gamma");
    select(&mut app, "clients[0]");
    app.update(Msg::Key(KeyPress::Remove));
    assert_eq!(pane(&app).edits().len(), 1, "the edit is folded in");
    let written = close_with(&mut app, &entries_dog_section());
    let expected = entries_dog_section().replace(ALPHA, "").replace(
        "key = \"hunter4\"\n",
        "key = \"hunter4\"\nname = \"gamma\"\n",
    );
    assert_eq!(written, expected);
}

#[test]
fn u_after_a_removal_gives_the_array_back_and_esc_writes_nothing() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    select(&mut app, "clients[0]");
    app.update(Msg::Key(KeyPress::Remove));
    app.update(Msg::Key(KeyPress::Undo));
    assert!(pane(&app).edits().is_empty());
    assert_eq!(pane(&app).edited_value("clients"), None);
    select(&mut app, "clients[2]");
    assert_eq!(
        app.update(Msg::Key(KeyPress::Escape)),
        Effect::None,
        "nothing is filed, so the pane closes without a write"
    );
    assert!(app.config_pane().is_none());
}

#[test]
fn d_on_an_entry_is_refused_on_a_read_only_lookout() {
    let mut app = fixtures::app_in_read_only_dog_pane_with_entries();
    open(&mut app, "clients");
    app.update(Msg::Key(KeyPress::SelectDown));
    assert!(pane(&app).cursor_on_entry());
    assert_eq!(app.update(Msg::Key(KeyPress::Remove)), Effect::None);
    assert!(pane(&app).edits().is_empty());
    assert_eq!(
        app.notice().map(ToString::to_string),
        Some(READ_ONLY_REFUSAL.to_string())
    );
}

#[test]
fn d_on_an_entries_row_with_no_entries_does_nothing() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    refresh_with(&mut app, "retries = 3\n".to_owned());
    select(&mut app, "clients");
    assert_eq!(app.update(Msg::Key(KeyPress::Remove)), Effect::None);
    assert!(pane(&app).edits().is_empty());
}

#[test]
fn d_on_a_row_inside_an_entry_restores_that_fields_default() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    select(&mut app, "clients[0].timeout");
    assert_eq!(app.update(Msg::Key(KeyPress::Remove)), Effect::None);
    let pane = pane(&app);
    assert_eq!(pane.edits().len(), 1);
    assert!(
        pane.edits()
            .get(&EditKey::Field("clients[0].timeout".into()))
            .is_some(),
        "the field is unset and the entry stays"
    );
    assert_eq!(pane.value("clients"), "3 entries");
}

#[test]
fn the_cursor_lands_on_the_entry_that_follows_the_removed_one() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    select(&mut app, "clients[1]");
    app.update(Msg::Key(KeyPress::Remove));
    assert_eq!(cursor_label(&app), "#1");
    let open = pane(&app).open_tables();
    assert!(open.contains("clients[1]"), "{open:?}");
    assert!(!open.contains("clients[2]"), "{open:?}");
}

#[test]
fn the_cursor_lands_on_the_previous_entry_after_removing_the_last() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    select(&mut app, "clients[2]");
    app.update(Msg::Key(KeyPress::Remove));
    assert_eq!(cursor_label(&app), "b-2");
}

#[test]
fn removing_every_entry_leaves_the_cursor_on_the_entries_row_and_writes_no_tables() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    for _ in 0..3 {
        select(&mut app, "clients[0]");
        app.update(Msg::Key(KeyPress::Remove));
    }
    let pane_ref = pane(&app);
    let Some(PaneRow::Field(field)) = pane_ref.cursor() else {
        panic!("the cursor is on a field: {:?}", pane_ref.cursor());
    };
    assert_eq!(pane_ref.fields().fields()[field].key, "clients");
    let written = close_with(&mut app, &entries_dog_section());
    assert!(!written.contains("clients"), "{written}");
    assert!(written.contains("[models.large]"), "{written}");
}

#[test]
fn removing_a_model_removes_its_table_and_leaves_the_other() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open(&mut app, "models");
    app.update(Msg::Key(KeyPress::SelectDown));
    assert_eq!(cursor_label(&app), "large");
    app.update(Msg::Key(KeyPress::Remove));
    assert_eq!(cursor_label(&app), "small");
    let written = close_with(&mut app, &entries_dog_section());
    assert_eq!(
        written,
        entries_dog_section().replace("[models.large]\nprovider = \"remote\"\n\n", "")
    );
}
