//! Fixtures more than one of `app`'s test modules leans on.

use serde_json::json;
use shep_core::config::AppConfig;

use super::*;
pub(super) use crate::lookout::edits::EditKey;
pub(super) use crate::lookout::view::fixtures;

pub(super) fn sheep(id: u32, name: &str, status: ProcStatus) -> ProcessInfo {
    ProcessInfo::builder(id, name, status)
        .pid(Some(1000 + id))
        .uptime_ms(60_000)
        .build()
}

pub(super) fn started() -> (App, Instant) {
    let t0 = Instant::now();
    let mut app = App::new(
        Palette::detect(None, None, None),
        Control::ReadOnly,
        "/home/ada/.shep".to_string(),
        t0,
    );
    app.update(Msg::Snapshot {
        rows: vec![
            sheep(1, "web", ProcStatus::Online),
            sheep(2, "api", ProcStatus::Errored),
            sheep(3, "worker", ProcStatus::Online),
        ],
        at: t0,
    });
    (app, t0)
}

/// `started()`'s three sheep with the gate open and the cursor mid-list, on
/// `web` at id 1.
///
/// The table reads by name, so the ids disagree with the display order:
/// `api` 2, `web` 1, `worker` 3. Mid-list, because a cursor clamped at
/// either end would pass the tests that assert a stray `j` did not move it.
pub(super) fn allowed() -> App {
    let t0 = Instant::now();
    let mut app = App::new(
        Palette::detect(None, None, None),
        Control::Allowed,
        "/home/ada/.shep".to_string(),
        t0,
    );
    app.update(Msg::Snapshot {
        rows: vec![
            sheep(1, "web", ProcStatus::Online),
            sheep(2, "api", ProcStatus::Online),
            sheep(3, "worker", ProcStatus::Online),
        ],
        at: t0,
    });
    app.update(Msg::Tick { now: t0 });
    app.update(Msg::Key(KeyPress::SelectDown));
    app
}

/// `allowed()`'s shape with three instances of one app: `web` at slots 0, 1
/// and 2, ids 1 through 3. Nothing is selected; each test selects itself.
pub(super) fn allowed_with_instances() -> App {
    let t0 = Instant::now();
    let mut app = App::new(
        Palette::detect(None, None, None),
        Control::Allowed,
        "/home/ada/.shep".to_string(),
        t0,
    );
    app.update(Msg::Snapshot {
        rows: instanced_rows(),
        at: t0,
    });
    app
}

/// `web`'s three instances, at slots 0, 1 and 2 and ids 1 through 3.
pub(super) fn instanced_rows() -> Vec<ProcessInfo> {
    (0..3)
        .map(|slot| {
            ProcessInfo::builder(slot + 1, "web", ProcStatus::Online)
                .instance(Some(slot))
                .build()
        })
        .collect()
}

/// The request an effect would put on the wire, or a panic naming what
/// came back instead. The seam this test module cares about: the
/// reducer's own `Sent` is an echo tag, and `Sent::request` is what the
/// link task actually sends.
pub(super) fn wire(effect: Effect) -> Request {
    match effect {
        Effect::Send(sent) => sent.request(),
        other => panic!("expected a request, got {other:?}"),
    }
}

/// A dashboard whose filter is set without any keymap involved.
///
/// Four sheep, two of which contain `web`: `api-web` at id 1 and
/// `web-worker` at id 4, with `cron` and `queue` between them. The table
/// sorts by name, so the gap is what makes `j` stepping over a hidden row
/// falsifiable.
pub(super) fn filtered(query: &str) -> App {
    let t0 = Instant::now();
    let mut app = App::new(
        Palette::detect(None, None, None),
        Control::ReadOnly,
        "/home/ada/.shep".to_string(),
        t0,
    );
    app.update(Msg::Snapshot {
        rows: vec![
            sheep(1, "api-web", ProcStatus::Online),
            sheep(2, "cron", ProcStatus::Online),
            sheep(3, "queue", ProcStatus::Online),
            sheep(4, "web-worker", ProcStatus::Online),
        ],
        at: t0,
    });
    app.set_filter(TextBox::from(query));
    app
}

/// Walks the pane's cursor onto `key`. The pane is a public type with
/// no public "go to this field" key, so the cursor is driven the way
/// an operator drives it. A thin wrapper: `view::fixtures::select_field`
/// is this exact walk, and this module had its own copy before the tab
/// row gave a field's group somewhere to switch to first.
pub(super) fn pane_to(app: &mut App, key: &str) {
    fixtures::select_field(app, key);
}

/// Lands a `Request::SheepConfig` reply for `web` carrying `env_keys`,
/// the way the event loop lands one after a write or an `r`.
pub(super) fn refresh_config(app: &mut App, env_keys: &[&str]) {
    let mut config = shep_core::config::AppConfig {
        name: "web".to_string(),
        ..Default::default()
    };
    for key in env_keys {
        config.env.insert((*key).to_string(), "x".to_string());
    }
    app.update(Msg::Replied {
        sent: Sent::SheepConfig {
            name: "web".to_string(),
        },
        result: Ok(Response::SheepConfig(Box::new(
            shep_core::protocol::SheepConfigView::new(config, Vec::new(), Vec::new()),
        ))),
    });
}

/// `esc`, and `c` right behind it if that raised the close dialog
/// instead of writing outright.
///
/// What every write-on-close test in this module wants now: every
/// sheep fixture here parks `kill_signal` unconditionally
/// (`sheep_config_view`'s own default), so a bare `esc` only asks. `c`
/// is what actually gets the write onto the wire, the same as an
/// operator continuing past the dialog would; harmless when nothing
/// asked, since [`App::close_dialog`] is `None` and this returns
/// `esc`'s own effect unchanged.
pub(super) fn close_writing(app: &mut App) -> Effect {
    let effect = app.update(Msg::Key(KeyPress::Escape));
    if app.close_dialog().is_some() {
        app.update(Msg::Key(KeyPress::Continue))
    } else {
        effect
    }
}

/// Every request a closed pane's batch would put on the wire, in the
/// order it sends them.
pub(super) fn wire_all(effect: Effect) -> Vec<Request> {
    match effect {
        Effect::SendAll(batch) => batch.iter().map(Sent::request).collect(),
        other => panic!("expected a batch, got {other:?}"),
    }
}

/// The `Sent` values a closed pane's batch carries.
pub(super) fn wire_batch(effect: Effect) -> Vec<Sent> {
    match effect {
        Effect::SendAll(batch) => batch,
        other => panic!("expected a batch, got {other:?}"),
    }
}

/// The one request a closed pane's batch carries, or a panic naming
/// how many it carried instead.
pub(super) fn one_wire(effect: Effect) -> Request {
    let mut requests = wire_all(effect);
    assert_eq!(requests.len(), 1, "{requests:?}");
    requests.remove(0)
}

/// The value the open pane has filed for `key`.
pub(super) fn filed_value(app: &App, key: &str) -> serde_json::Value {
    let entry = app
        .config_pane()
        .expect("the pane is open")
        .edits()
        .get(&EditKey::Field(key.to_owned()))
        .unwrap_or_else(|| panic!("nothing filed for {key}"));
    match entry.edit() {
        PaneEdit::Set { value, .. } => value.as_value().clone(),
        other => panic!("expected a field set, got {other:?}"),
    }
}

/// A secret the `jobs` table holds, and one the schema-less `legacy`
/// table holds. Neither may reach a notice, a `Debug` or a row.
pub(super) const TOKEN: &str = "sk-live-51Hx9Qa";
pub(super) const PASSWORD: &str = "hunter2-legacy";

pub(super) fn web_view(with_jobs: bool) -> SheepConfigView {
    let mut config = AppConfig {
        name: "web".into(),
        ..AppConfig::default()
    };
    let table = |value: serde_json::Value| value.as_object().cloned().expect("a table");
    if with_jobs {
        config.dogs.insert(
            "jobs".into(),
            table(json!({ "concurrency": 2, "token": TOKEN })).into(),
        );
    }
    config.dogs.insert(
        "legacy".into(),
        table(json!({ "url": format!("https://ops:{PASSWORD}@example.test") })).into(),
    );
    SheepConfigView::new(config, Vec::new(), Vec::new())
}

pub(super) fn app_in_web(control: Control) -> App {
    let mut app = fixtures::with_selection(
        ProcessInfo::builder(9, "web", ProcStatus::Online)
            .pid(Some(48_000))
            .build(),
    );
    app.set_control_for_tests(control);
    let _ = app.update(Msg::Key(KeyPress::Edit));
    let _ = app.update(Msg::Replied {
        sent: Sent::SheepConfig { name: "web".into() },
        result: Ok(Response::SheepConfig(Box::new(web_view(true)))),
    });
    pane_to(&mut app, "dogs");
    app
}

pub(super) fn jobs_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "concurrency": { "type": "integer" },
            "token": { "type": "string", "x-shep-secret": true },
        },
    })
}

pub(super) fn probe() -> Vec<SheepDogEntry> {
    vec![
        SheepDogEntry {
            name: "jobs".into(),
            adopted_path: Some("/opt/jobs".into()),
            schema: Some(jobs_schema()),
        },
        SheepDogEntry {
            name: "deploy".into(),
            adopted_path: Some("/opt/deploy".into()),
            schema: Some(json!({ "type": "object", "properties": {} })),
        },
        SheepDogEntry {
            name: "legacy".into(),
            adopted_path: None,
            schema: None,
        },
    ]
}

/// `key` on `web`'s dogs row, and the ask the probe it raised carries.
pub(super) fn ask_dogs(app: &mut App, key: KeyPress) -> u64 {
    match app.update(Msg::Key(key)) {
        Effect::LoadSheepDogs { sheep, ask } if sheep == "web" => ask,
        other => panic!("no probe for web: {other:?}"),
    }
}

/// The probe's answer for `sheep`, as ask `ask`.
pub(super) fn answer(app: &mut App, sheep: &str, ask: u64) {
    let _ = app.update(Msg::SheepDogs {
        sheep: sheep.into(),
        ask,
        dogs: probe(),
    });
}

/// `app_in_web`, with the probe answered and the sub-screen up. Rows
/// sort by name: `deploy`, `jobs`, `legacy`.
pub(super) fn app_in_dogs(control: Control) -> App {
    let mut app = app_in_web(control);
    let ask = ask_dogs(&mut app, KeyPress::Confirm);
    answer(&mut app, "web", ask);
    app
}

pub(super) fn dogs(app: &App) -> &DogsPane {
    app.config_pane()
        .and_then(ConfigPane::dogs)
        .expect("the sub-screen is up")
}

pub(super) fn cursor_on(app: &mut App, dog: &str) {
    let _ = app.update(Msg::Key(KeyPress::SelectFirst));
    while dogs(app).cursor_row().map(|row| row.name()) != Some(dog) {
        let before = dogs(app).view().cursor();
        let _ = app.update(Msg::Key(KeyPress::SelectDown));
        assert_ne!(dogs(app).view().cursor(), before, "{dog} is not listed");
    }
}

/// `app_in_dogs`, with `jobs`'s table pane open over `web`.
pub(super) fn app_in_jobs_table() -> App {
    let mut app = app_in_dogs(Control::Allowed);
    cursor_on(&mut app, "jobs");
    let _ = app.update(Msg::Key(KeyPress::Confirm));
    assert!(matches!(
        app.config_pane().map(ConfigPane::target),
        Some(PaneTarget::SheepDog { .. })
    ));
    app
}

pub(super) fn type_concurrency(app: &mut App, typed: &str) {
    pane_to(app, "concurrency");
    let _ = app.update(Msg::Key(KeyPress::Confirm));
    let _ = app.update(Msg::Key(KeyPress::TextBackspace));
    for c in typed.chars() {
        let _ = app.update(Msg::Key(KeyPress::TextChar(c)));
    }
    let _ = app.update(Msg::Key(KeyPress::TextApply));
}

/// `esc` on [`fixtures::app_in_dog_pane`]'s pane, with the re-read it
/// sends answered by the section the pane opened on.
pub(super) fn close_bark(app: &mut App) -> Effect {
    assert_eq!(
        app.update(Msg::Key(KeyPress::Escape)),
        Effect::Send(Sent::DogSection {
            name: "bark".into()
        }),
        "esc reads the section again before writing"
    );
    app.update(Msg::Replied {
        sent: Sent::DogSection {
            name: "bark".into(),
        },
        result: Ok(Response::DogSection {
            toml: fixtures::dog_section().into(),
        }),
    })
}

/// `esc` on [`app_in_jobs_table`]'s pane, with the re-read it sends
/// answered by the table the pane opened on: the effect carrying the write.
pub(super) fn close_jobs_table(app: &mut App) -> Effect {
    assert_eq!(
        app.update(Msg::Key(KeyPress::Escape)),
        Effect::Send(Sent::SheepConfig { name: "web".into() }),
        "esc reads the sheep again before writing"
    );
    app.update(Msg::Replied {
        sent: Sent::SheepConfig { name: "web".into() },
        result: Ok(Response::SheepConfig(Box::new(web_view(true)))),
    })
}
