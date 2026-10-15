//! The table rendering of a bleats line: the stream colour a plain line
//! gets, the line that already carries its own, and the renderings that
//! add nothing. Driven through the two call sites directly, the way
//! [`tail`] drives the backlog path, so a followed line and a replayed
//! line are compared at the same seam.

use std::collections::HashMap;
use std::ffi::OsStr;

use crate::style::{Presentation, StyleLevel};

use super::*;

/// Colour on, 16-colour tier: what `full` at a plain terminal resolves to.
fn coloured() -> Presentation {
    Presentation::new(StyleLevel::Full, None, None, None, 80)
}

/// Feeds `events` through [`handle_event`] for one sheep named `web` and
/// hands back what reached stdout.
fn render_followed(style: Presentation, fmt: Format, events: Vec<BusEvent>) -> String {
    let cache: HashMap<u32, ProcessInfo> = [(1u32, info(1, "web"))].into_iter().collect();
    let counts = instance_counts(&cache);
    let selector = ProcessSelector::parse("all").expect("selector");
    let args = follow_args("all");

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style,
            fmt,
        };
        for event in events {
            handle_event(
                &mut streams,
                false,
                &cache,
                &counts,
                &selector,
                &args,
                event,
            )
            .expect("a Vec sink never fails a write");
        }
    }
    String::from_utf8(out).unwrap()
}

#[test]
fn stdout_is_green_and_stderr_is_red_when_colour_is_on() {
    let rendered = render_followed(
        coloured(),
        Format::Table,
        vec![
            BusEvent::LogOut {
                id: 1,
                line: "to-stdout".into(),
            },
            BusEvent::LogErr {
                id: 1,
                line: "to-stderr".into(),
            },
        ],
    );
    assert_eq!(
        rendered, "web | \u{1b}[32mto-stdout\u{1b}[0m\nweb | \u{1b}[31mto-stderr\u{1b}[0m\n",
        "the `name |` label stays plain and only the line text is wrapped: {rendered:?}"
    );
}

#[test]
fn a_pipe_no_color_and_json_add_no_colour() {
    let events = || {
        vec![BusEvent::LogOut {
            id: 1,
            line: "hello".into(),
        }]
    };

    // `bare` is what a non-terminal stdout resolves to.
    let rendered = render_followed(Presentation::BARE, Format::Table, events());
    assert_eq!(rendered, "web | hello\n");
    assert!(
        !rendered.contains('\u{1b}'),
        "a pipe gets the line exactly as it always did: {rendered:?}"
    );

    // `full` under `NO_COLOR` keeps the layout and drops the colour.
    let vetoed = Presentation::new(StyleLevel::Full, Some(OsStr::new("1")), None, None, 80);
    let rendered = render_followed(vetoed, Format::Table, events());
    assert_eq!(
        rendered, "web | hello\n",
        "NO_COLOR must veto the stream colour too: {rendered:?}"
    );

    // `--format json` carries the sheep's own bytes, sanitiser and all.
    let rendered = render_followed(
        coloured(),
        Format::Json,
        vec![BusEvent::LogErr {
            id: 1,
            line: "\u{1b}[31mraw\u{1b}[0m".into(),
        }],
    );
    let json: serde_json::Value = serde_json::from_str(rendered.lines().next().unwrap()).unwrap();
    assert_eq!(
        json["line"], "\u{1b}[31mraw\u{1b}[0m",
        "the JSON line is the sheep's own bytes: {rendered}"
    );

    // A line that already colours itself keeps its own colour and gets no
    // default wrapped around it, even with colour on.
    let rendered = render_followed(
        coloured(),
        Format::Table,
        vec![BusEvent::LogOut {
            id: 1,
            line: "\u{1b}[35mpink\u{1b}[0m".into(),
        }],
    );
    assert_eq!(
        rendered, "web | \u{1b}[35mpink\u{1b}[0m\n",
        "the process's own colour is kept, not overpainted: {rendered:?}"
    );
}

#[test]
fn a_followed_line_and_a_replayed_line_render_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let out_path = write_log(
        dir.path(),
        "web-out.log",
        "plain-line\n\u{1b}[31mred-line\u{1b}[0m\n",
    );

    let mut sheep = info(1, "web");
    sheep.out_file = Some(out_path);
    let cache: HashMap<u32, ProcessInfo> = [(1u32, sheep)].into_iter().collect();
    let selector = ProcessSelector::parse("all").expect("selector");

    // The replayed half: the backlog a follow prints before subscribing.
    let mut backlog = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = Streams {
            out: &mut backlog,
            err: &mut err,
            style: coloured(),
            fmt: Format::Table,
        };
        tail_log_files(
            &mut streams,
            false,
            &cache,
            &selector,
            &follow_args_out("all"),
        );
    }

    // The followed half: the same two lines arriving on the bus.
    let followed = render_followed(
        coloured(),
        Format::Table,
        vec![
            BusEvent::LogOut {
                id: 1,
                line: "plain-line".into(),
            },
            BusEvent::LogErr {
                id: 1,
                line: "\u{1b}[31mred-line\u{1b}[0m".into(),
            },
        ],
    );

    assert_eq!(
        String::from_utf8(backlog)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        followed.lines().collect::<Vec<_>>(),
        "the backlog half and the followed half must agree byte for byte"
    );
    assert_eq!(
        followed, "web | \u{1b}[32mplain-line\u{1b}[0m\nweb | \u{1b}[31mred-line\u{1b}[0m\n",
        "and the shared rendering is the stream colour, with a line that \
         colours itself left alone: {followed:?}"
    );
}
