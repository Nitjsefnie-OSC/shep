//! `Enter` on `+ new` under a dog pane's array or map of tables, driven by
//! key presses: what it asks, what it files, and what the gate refuses.

use super::dog_pane_entries::{close_with, open, open_every_client, pane, select, type_into};
use super::*;
use crate::lookout::app::testing::*;
use crate::lookout::view::fixtures::{entries_dog_section, render, rows_of};

/// `Enter` on the `+ new` row of `key`, which is opened first.
#[track_caller]
fn press_new(app: &mut App, key: &str) {
    open(app, key);
    select(app, &format!("{key}[+]"));
    app.update(Msg::Key(KeyPress::Confirm));
}

/// Types `text` into the open key editor and applies it.
#[track_caller]
fn name_it(app: &mut App, text: &str) {
    for typed in text.chars() {
        app.update(Msg::Key(KeyPress::TextChar(typed)));
    }
    app.update(Msg::Key(KeyPress::TextApply));
}

const NEW_CLIENT: &str = "[[clients]]\nenabled = true\nkey = \"change-me\"\n\n";

#[test]
fn enter_on_plus_new_in_an_array_appends_at_once_and_sends_nothing_until_esc() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open(&mut app, "clients");
    select(&mut app, "clients[+]");
    assert_eq!(app.update(Msg::Key(KeyPress::Confirm)), Effect::None);
    assert_eq!(app.mode(), InputMode::Normal, "an array asks for nothing");
    assert_eq!(pane(&app).edits().len(), 1);
    assert_eq!(
        pane(&app).edited_value("clients").as_deref(),
        Some("4 entries")
    );
    let written = close_with(&mut app, &entries_dog_section());
    let want =
        entries_dog_section().replace("[models.large]", &format!("{NEW_CLIENT}[models.large]"));
    assert_eq!(written, want);
}

#[test]
fn the_new_client_opens_and_the_cursor_lands_on_its_first_field() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open(&mut app, "clients");
    select(&mut app, "clients[+]");
    app.update(Msg::Key(KeyPress::Confirm));
    let pane = pane(&app);
    assert!(pane.open_tables().contains("clients[3]"));
    let text = rows_of(&render(&app, 160, 48)).join("\n");
    let cursor_row = text
        .lines()
        .find(|line| line.starts_with('>'))
        .expect("the cursor is drawn");
    assert!(
        cursor_row.contains("enabled") && cursor_row.contains("true"),
        "{cursor_row}"
    );
}

#[test]
fn a_secret_default_reads_set_in_the_tree_and_never_prints() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open(&mut app, "clients");
    select(&mut app, "clients[+]");
    app.update(Msg::Key(KeyPress::Confirm));
    let text = rows_of(&render(&app, 160, 48)).join("\n");
    assert!(
        text.lines()
            .any(|line| line.contains("key") && line.contains("<set>")),
        "{text}"
    );
    assert!(!text.contains("change-me"), "{text}");
}

#[test]
fn enter_on_plus_new_in_a_map_opens_the_key_editor_in_text_mode() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    press_new(&mut app, "models");
    assert_eq!(app.mode(), InputMode::Text);
    assert!(
        pane(&app).edits().is_empty(),
        "nothing is filed until a key is applied"
    );
    let text = rows_of(&render(&app, 160, 48)).join("\n");
    assert!(text.contains("new models key"), "{text}");
}

#[test]
fn a_key_applied_adds_the_model_and_closes_the_editor() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    press_new(&mut app, "models");
    name_it(&mut app, "gpt-5");
    assert_eq!(app.mode(), InputMode::Normal);
    assert_eq!(pane(&app).edits().len(), 1);
    assert!(pane(&app).open_tables().contains("models[\"gpt-5\"]"));
    let written = close_with(&mut app, &entries_dog_section());
    assert_eq!(
        written,
        entries_dog_section() + "\n[models.gpt-5]\ncontext = 4096\n"
    );
}

#[test]
fn a_key_with_a_dot_is_written_quoted() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    press_new(&mut app, "models");
    name_it(&mut app, "a.b");
    let written = close_with(&mut app, &entries_dog_section());
    assert!(
        written.contains("[models.\"a.b\"]\ncontext = 4096\n"),
        "{written}"
    );
}

#[test]
fn a_duplicate_key_is_refused_by_name_and_the_editor_stays_open() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    press_new(&mut app, "models");
    name_it(&mut app, "large");
    assert_eq!(app.mode(), InputMode::Text, "the editor stays open");
    assert!(pane(&app).edits().is_empty());
    let notice = app.notice().expect("a refusal is raised");
    assert!(notice.to_string().contains("\"large\""), "{notice}");
    assert!(pane(&app).typing().is_some());
}

#[test]
fn an_empty_key_abandons_and_writes_nothing() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    press_new(&mut app, "models");
    app.update(Msg::Key(KeyPress::TextApply));
    assert_eq!(app.mode(), InputMode::Normal);
    assert!(pane(&app).edits().is_empty());
    assert_eq!(app.update(Msg::Key(KeyPress::Escape)), Effect::None);
    assert!(
        app.config_pane().is_none(),
        "nothing filed, so the pane closes"
    );
}

#[test]
fn an_added_client_then_an_edit_inside_it_is_one_write_with_both() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open(&mut app, "clients");
    select(&mut app, "clients[+]");
    app.update(Msg::Key(KeyPress::Confirm));
    type_into(&mut app, "clients[3].name", "delta");
    let written = close_with(&mut app, &entries_dog_section());
    assert_eq!(written.matches("[[clients]]").count(), 4, "{written}");
    assert!(
        written.contains("[[clients]]\nenabled = true\nkey = \"change-me\"\nname = \"delta\"\n"),
        "{written}"
    );
}

#[test]
fn adding_then_removing_the_same_client_writes_nothing() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open(&mut app, "clients");
    select(&mut app, "clients[+]");
    app.update(Msg::Key(KeyPress::Confirm));
    select(&mut app, "clients[3]");
    app.update(Msg::Key(KeyPress::Remove));
    assert!(pane(&app).edits().is_empty());
    assert_eq!(app.update(Msg::Key(KeyPress::Escape)), Effect::None);
    assert!(app.config_pane().is_none());
}

#[test]
fn an_add_keeps_the_edits_already_filed_on_the_array() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open_every_client(&mut app);
    type_into(&mut app, "clients[0].name", "zed");
    assert_eq!(pane(&app).edits().len(), 1);
    select(&mut app, "clients[+]");
    app.update(Msg::Key(KeyPress::Confirm));
    assert_eq!(
        pane(&app).edits().len(),
        1,
        "folded into the one whole value"
    );
    let written = close_with(&mut app, &entries_dog_section());
    assert!(written.contains("name = \"zed\""), "{written}");
    assert!(!written.contains("alpha"), "{written}");
    assert_eq!(written.matches("[[clients]]").count(), 4, "{written}");
}

#[test]
fn u_after_an_add_gives_the_array_back() {
    let mut app = fixtures::app_in_dog_pane_with_entry_defaults();
    open(&mut app, "clients");
    select(&mut app, "clients[+]");
    app.update(Msg::Key(KeyPress::Confirm));
    app.update(Msg::Key(KeyPress::Undo));
    assert!(pane(&app).edits().is_empty());
    assert_eq!(pane(&app).edited_value("clients"), None);
}

#[test]
fn plus_new_is_refused_on_a_read_only_lookout() {
    for key in ["clients", "models"] {
        let mut app = fixtures::app_in_read_only_dog_pane_with_entries();
        press_new(&mut app, key);
        assert_eq!(app.mode(), InputMode::Normal, "{key}");
        assert!(
            pane(&app).edits().is_empty() && pane(&app).typing().is_none(),
            "{key}"
        );
        assert_eq!(
            app.notice().map(ToString::to_string),
            Some(READ_ONLY_REFUSAL.to_string()),
            "{key}"
        );
    }
}
