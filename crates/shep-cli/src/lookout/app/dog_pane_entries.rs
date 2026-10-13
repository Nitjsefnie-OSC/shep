//! A dog pane's arrays and maps of tables, driven by key presses: how they
//! open, what the keys that edit do on them, and what a refresh keeps.

use super::*;
use crate::lookout::app::testing::*;
use crate::lookout::pane::EntryBody;
use crate::lookout::view::fixtures::{entries_dog_section, render, rows_of};

fn pane(app: &App) -> &ConfigPane {
    app.config_pane().expect("the pane is open")
}

/// Presses `→` on the row whose key is `key`, walking down to it first.
fn open(app: &mut App, key: &str) {
    app.update(Msg::Key(KeyPress::SelectFirst));
    while !matches!(
        pane(app).cursor(),
        Some(PaneRow::Field(index)) if pane(app).fields().fields()[index].key == key
    ) {
        app.update(Msg::Key(KeyPress::SelectDown));
    }
    app.update(Msg::Key(KeyPress::TabNext));
}

/// Opens `clients` and every entry under it.
fn open_every_client(app: &mut App) {
    open(app, "clients");
    loop {
        let pane = pane(app);
        let rows = pane.rows();
        let closed = rows.iter().position(|row| match row {
            PaneRow::Entry(index) => pane.entry_row(*index).is_some_and(|entry| {
                matches!(&entry.body, EntryBody::Node { path, .. } if !pane.open_tables().contains(path))
            }),
            _ => false,
        });
        let Some(at) = closed else {
            return;
        };
        app.update(Msg::Key(KeyPress::SelectFirst));
        for _ in 0..at {
            app.update(Msg::Key(KeyPress::SelectDown));
        }
        app.update(Msg::Key(KeyPress::TabNext));
    }
}

#[test]
fn entries_and_entry_rows_start_closed() {
    let app = fixtures::app_in_dog_pane_with_entries();
    let pane = pane(&app);
    assert_eq!(pane.rows().len(), 3, "clients, models and retries");
    assert!(pane.open_tables().is_empty(), "{:?}", pane.open_tables());
}

#[test]
fn a_frame_with_every_client_open_counts_them_and_never_draws_a_credential() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    let text = rows_of(&render(&app, 160, 48)).join("\n");
    assert!(text.contains("3 entries"), "{text}");
    assert!(text.contains("<set>"), "{text}");
    for secret in ["hunter2", "hunter3", "hunter4"] {
        assert!(
            !text.contains(secret),
            "{secret} reached the frame:\n{text}"
        );
    }
}

/// Every row under an entry is read-only, so `e`, space and `d` on it file
/// nothing and open no editor.
#[test]
fn e_space_and_d_on_a_row_under_an_entry_file_nothing() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    let leaves: Vec<usize> = pane(&app)
        .rows()
        .iter()
        .enumerate()
        .filter_map(|(at, row)| match row {
            PaneRow::Entry(index) => pane(&app)
                .entry_row(*index)
                .is_some_and(|row| matches!(row.body, EntryBody::Leaf { .. }))
                .then_some(at),
            _ => None,
        })
        .collect();
    assert!(leaves.len() >= 4, "{leaves:?}");
    for at in leaves {
        for key in [
            KeyPress::Edit,
            KeyPress::Cycle,
            KeyPress::Remove,
            KeyPress::Confirm,
        ] {
            app.update(Msg::Key(KeyPress::SelectFirst));
            for _ in 0..at {
                app.update(Msg::Key(KeyPress::SelectDown));
            }
            assert_eq!(
                pane(&app).cursor_lock().map(|(_, lock)| lock),
                Some(Lock::NoWidget),
                "row {at}"
            );
            assert_eq!(app.update(Msg::Key(key)), Effect::None, "{key:?} at {at}");
            let pane = pane(&app);
            assert!(pane.edits().is_empty(), "{key:?} at {at} filed something");
            assert!(pane.typing().is_none(), "{key:?} at {at} opened an editor");
        }
    }
}

#[test]
fn a_refresh_keeps_an_open_map_entry_open_by_key_after_a_key_above_it_is_removed() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open(&mut app, "models");
    // `large` is first, `small` second.
    app.update(Msg::Key(KeyPress::SelectDown));
    app.update(Msg::Key(KeyPress::SelectDown));
    app.update(Msg::Key(KeyPress::TabNext));
    assert!(pane(&app).open_tables().contains("models[\"small\"]"));
    let Effect::Send(sent) = app.update(Msg::Key(KeyPress::Refresh)) else {
        panic!("r re-reads the section");
    };
    let without_large =
        entries_dog_section().replace("[models.large]\nprovider = \"remote\"\n\n", "");
    app.update(Msg::Replied {
        sent,
        result: Ok(Response::DogSection {
            toml: without_large.into(),
        }),
    });
    let open: Vec<&String> = pane(&app).open_tables().iter().collect();
    assert_eq!(open, ["models", "models[\"small\"]"]);
}

#[test]
fn a_refresh_drops_an_open_array_entry_past_the_new_end() {
    let mut app = fixtures::app_in_dog_pane_with_entries();
    open_every_client(&mut app);
    assert!(pane(&app).open_tables().contains("clients[2]"));
    let Effect::Send(sent) = app.update(Msg::Key(KeyPress::Refresh)) else {
        panic!("r re-reads the section");
    };
    let two = entries_dog_section().replace("[[clients]]\nkey = \"hunter4\"\n\n", "");
    app.update(Msg::Replied {
        sent,
        result: Ok(Response::DogSection { toml: two.into() }),
    });
    let open = pane(&app).open_tables();
    assert!(open.contains("clients[1]"), "{open:?}");
    assert!(!open.contains("clients[2]"), "{open:?}");
}
