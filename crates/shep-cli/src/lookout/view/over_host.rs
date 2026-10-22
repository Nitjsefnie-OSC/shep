//! A lookout over `--host`: the title names the host, the strip is absent and
//! every feed says it is not available.

use super::fixtures::{self, flock_of, sample};
use super::testing::draw_to;
use crate::lookout::app::{App, KeyPress, Msg};

fn watching_build_01(mut app: App) -> App {
    app.set_remote_host("build-01");
    app
}

/// The dashboard: the title band names the host in place of `$SHEP_HOME`, the
/// strip row stays blank rather than claiming the platform is unsupported,
/// and the feed beside the table says why it is empty.
#[test]
fn a_host_dashboard_names_the_host_and_draws_no_strip_and_no_feed() {
    let app = watching_build_01(fixtures::with_host(sample(), flock_of(3, 0)));

    let frame = draw_to(&app, 160, 46);

    let title = frame.lines().next().unwrap_or_default();
    assert!(title.contains("shep lookout   build-01"), "{frame}");
    assert!(!title.contains("/home/ada/.shep"), "{frame}");
    assert!(!frame.contains("host  load"), "{frame}");
    assert!(!frame.contains("unsupported"), "{frame}");
    assert!(!frame.contains("out then err"), "{frame}");
    assert!(frame.contains("not available over --host"), "{frame}");
}

/// The same sample on this machine's own lookout still draws the strip, so the
/// assertions above are about the host and not about the fixture.
#[test]
fn a_local_dashboard_still_draws_its_strip() {
    let app = fixtures::with_host(sample(), flock_of(3, 0));

    let frame = draw_to(&app, 160, 46);

    let title = frame.lines().next().unwrap_or_default();
    assert!(frame.contains("host  load"), "{frame}");
    assert!(title.contains("shep lookout   /home/ada/.shep"), "{frame}");
}

/// The full-screen pane draws its one line and none of the lines the feed
/// holds.
#[test]
fn the_full_screen_bleats_pane_says_it_is_not_available_over_a_host() {
    let app = watching_build_01(fixtures::bleats_pane_with_lines(5));

    let frame = draw_to(&app, 120, 30);

    assert!(frame.contains("not available over --host"), "{frame}");
    assert!(!frame.contains("line-0"), "{frame}");
}

/// The sheep pane's embedded column says the same.
#[test]
fn a_sheep_panes_feed_column_says_it_is_not_available_over_a_host() {
    let mut app = watching_build_01(fixtures::app_with(flock_of(2, 0), fixtures::plain()));
    app.update(Msg::Key(KeyPress::Confirm));

    let frame = draw_to(&app, 160, 46);

    assert!(frame.contains("not available over --host"), "{frame}");
    assert!(!frame.contains("out then err"), "{frame}");
}

/// The pane is told how many rows it has, and a host's two lines are no
/// exception on a pane one row tall.
#[test]
fn the_full_screen_pane_over_a_host_keeps_to_the_rows_it_is_given() {
    let app = watching_build_01(fixtures::bleats_pane_with_lines(5));

    for rows in 0..=2 {
        let lines = crate::lookout::view::bleats_full::draw_lines(&app, 80, rows);
        assert_eq!(lines.len(), rows.min(2), "{rows} rows");
    }
}
