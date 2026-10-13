//! A dog's config pane, and the TOML section behind it.

use shep_core::protocol::Response;

use crate::lookout::app::{App, Control, KeyPress, Msg, Sent};

use super::flock::{app_with, flock_of};
use super::palette::plain;
use super::settings::settings_snapshot;
use super::sheep_pane::select_field;

/// The bark dog's `[bark]` section as `Request::DogConfig` would answer it:
/// a comment, two scalars, and a sink holding a webhook credential.
///
/// The comment is load-bearing rather than decoration: a write goes out as
/// the WHOLE section, so a pane that re-rendered it from the parsed values
/// would delete this line on the operator's own keystroke.
pub fn dog_section() -> String {
    "# how often\npoll = \"60s\"\nhistory_bytes = 4096\n\n[sinks.ops]\nkind = \"slack\"\nurl = \"https://hooks.example/x\"\n"
        .to_string()
}

/// A dashboard with the bark dog's config pane open, opened the way the
/// event loop opens it: `e` on the settings screen's own dog row, then the
/// schema its binary answered with, then the shepherd's section. The
/// control gate is open, so the pane can write.
///
/// bark is [`settings_snapshot`]'s first dog, and the six scalar rows
/// always sort ahead of the dogs, so row 6 is its row.
pub fn app_in_dog_pane() -> App {
    let mut app = app_with(flock_of(3, 0), plain());
    app.set_control_for_tests(Control::Allowed);
    app.update(Msg::Key(KeyPress::Settings));
    app.update(Msg::Settings {
        result: Ok(settings_snapshot()),
    });
    for _ in 0..6 {
        app.update(Msg::Key(KeyPress::SelectDown));
    }
    app.update(Msg::Key(KeyPress::Edit));
    app.update(Msg::DogPane {
        name: "bark".to_string(),
        adopted_path: None,
        result: Ok(crate::dog::builtin_schema("bark").expect("bark is a built-in")),
    });
    app.update(Msg::Replied {
        sent: Sent::DogSection {
            name: "bark".to_string(),
        },
        result: Ok(Response::DogSection {
            toml: dog_section().into(),
        }),
    });
    app
}

/// A dog whose schema nests `models.worker` two tables down through a
/// `$ref` chain, with a credential beside a plain field in the inner one.
pub fn nested_dog_schema() -> serde_json::Value {
    serde_json::json!({
        "$ref": "#/$defs/Settings",
        "$defs": {
            "Settings": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "models": { "$ref": "#/$defs/Models" },
                },
            },
            "Models": {
                "type": "object",
                "properties": { "worker": { "$ref": "#/$defs/Worker" } },
            },
            "Worker": {
                "type": "object",
                "properties": {
                    "model": { "type": "string" },
                    "token": { "type": "string", "x-shep-secret": true },
                },
            },
        },
    })
}

/// The `[jobs]` section that schema opens on, in the header form of its
/// nested table.
pub fn nested_dog_section() -> String {
    "# which model to run\nname = \"small\"\n\n[models.worker]\n# the credential\ntoken = \"sk-live-51Hx9Qa\"\nmodel = \"gpt\"\n"
        .to_string()
}

/// A dashboard with the `jobs` dog's config pane open, over
/// [`nested_dog_section`], so a dotted row is what an edit walks. Opened the
/// way the event loop opens one: the schema probe, then the shepherd's
/// section. The control gate is open.
pub fn app_in_dog_pane_with_a_nested_table() -> App {
    let mut app = app_with(flock_of(3, 0), plain());
    app.set_control_for_tests(Control::Allowed);
    app.update(Msg::DogPane {
        name: "jobs".to_string(),
        adopted_path: None,
        result: Ok(nested_dog_schema()),
    });
    app.update(Msg::Replied {
        sent: Sent::DogSection {
            name: "jobs".to_string(),
        },
        result: Ok(Response::DogSection {
            toml: nested_dog_section().into(),
        }),
    });
    app
}

/// A dog whose schema holds a string list with an item pattern, an integer
/// list, a secret list and a map.
pub fn list_dog_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "hosts": {
                "type": "array",
                "items": { "type": "string", "minLength": 3, "pattern": "^[a-z.]+$" },
            },
            "keys": {
                "type": "array",
                "items": { "type": "string" },
                "x-shep-secret": true,
            },
            "labels": { "type": "object", "additionalProperties": { "type": "string" } },
            "ports": { "type": "array", "items": { "type": "integer" } },
        },
    })
}

/// The `[watch]` section [`list_dog_schema`] opens on: a comment above each
/// list, so a write that re-rendered the section would lose them.
pub fn list_dog_section() -> String {
    "# who to ping\nhosts = [\"a.example\", \"b.example\"]\n# the api keys\nkeys = [\"sk-live-51Hx9Qa\"]\nports = [80, 443]\n"
        .to_string()
}

/// A dashboard with the `watch` dog's config pane open over
/// [`list_dog_section`], opened the way the event loop opens one. The
/// control gate is open.
pub fn app_in_dog_pane_with_lists() -> App {
    let mut app = app_with(flock_of(3, 0), plain());
    app.set_control_for_tests(Control::Allowed);
    app.update(Msg::DogPane {
        name: "watch".to_string(),
        adopted_path: None,
        result: Ok(list_dog_schema()),
    });
    app.update(Msg::Replied {
        sent: Sent::DogSection {
            name: "watch".to_string(),
        },
        result: Ok(Response::DogSection {
            toml: list_dog_section().into(),
        }),
    });
    app
}

/// [`app_in_dog_pane`] with two edits filed, driven by real key presses:
/// `poll` typed, then `history_bytes` typed.
///
/// Two, and not one, because a batch of one cannot tell a loop from a
/// `take(1)`. What `closing_a_dog_pane_sends_one_write_for_two_edits`
/// needs: proof that a dog's batch is one `Sent::SetDogSection`, not two.
pub fn app_in_dog_pane_with_two_edits() -> App {
    let mut app = app_in_dog_pane();
    for (key, typed) in [("poll", "45s"), ("history_bytes", "8192")] {
        select_field(&mut app, key);
        app.update(Msg::Key(KeyPress::Confirm));
        for _ in 0..64 {
            app.update(Msg::Key(KeyPress::TextBackspace));
        }
        for character in typed.chars() {
            app.update(Msg::Key(KeyPress::TextChar(character)));
        }
        app.update(Msg::Key(KeyPress::TextApply));
    }
    assert_eq!(
        app.config_pane().expect("the pane is open").edits().len(),
        2,
        "the fixture files two edits"
    );
    app
}
