//! What a tolerated difference says: one notice line on stderr, stdout left to
//! the verb, and the collapse that keeps a peer-worded version off a terminal.

use super::*;

/// The shepherd's version is announced as `0.1.8`, which this build is not.
const SKEWED_WARNING: &str = concat!(
    "this shep is ",
    env!("CARGO_PKG_VERSION"),
    ", the running shepherd is 0.1.8. Listing works, but verbs that change the flock refuse \
     until it is reloaded. Run `shep daemon reload`."
);

#[tokio::test]
async fn a_read_verb_against_a_skewed_shepherd_proceeds_and_names_the_remedy() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        refuse_version_skew(&mut streams, &client, VersionGuard::Warn)
            .expect("a read verb proceeds against a skewed shepherd");
    }
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains(SKEWED_WARNING), "{text}");
    assert_eq!(text.lines().count(), 1, "one notice, one line: {text}");
}

#[tokio::test]
async fn under_json_the_skew_warning_is_one_notice_on_stderr_and_stdout_stays_empty() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        streams.fmt = Format::Json;
        refuse_version_skew(&mut streams, &client, VersionGuard::Warn).unwrap();
    }
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
    let text = String::from_utf8(err).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    let notice: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(notice["notice"]["code"], "version_skew");
    assert_eq!(notice["notice"]["message"], SKEWED_WARNING);
}

#[tokio::test]
async fn a_matching_version_warns_about_nothing_under_warn() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, env!("CARGO_PKG_VERSION")).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        refuse_version_skew(&mut streams, &client, VersionGuard::Warn).unwrap();
    }
    assert!(out.is_empty() && err.is_empty());
    assert_eq!(skew_of(&client).notice, None);
}

/// The version arrives over the socket, so a control character in it must
/// not reach the terminal.
#[tokio::test]
async fn a_version_with_an_escape_in_it_is_collapsed_in_the_warning() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8\u{1b}[31m").await;

    let text = skew_of(&client).notice.unwrap();
    assert!(!text.contains('\u{1b}'), "{text:?}");
}

#[tokio::test]
async fn flock_against_a_skewed_shepherd_lists_the_flock_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, _requests) = skewed_shepherd(dir.path(), ProcStatus::Online).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Warn,
            &flock_args(),
        )
        .await
    };

    assert_eq!(code, ExitCode::Success);
    let table = String::from_utf8(out).unwrap();
    assert!(table.contains("web") && table.contains("online"), "{table}");
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains(SKEWED_WARNING), "{text}");
}

#[tokio::test]
async fn flock_against_a_skewed_shepherd_lists_a_status_this_build_does_not_know() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, _requests) = skewed_shepherd(dir.path(), ProcStatus::Unrecognized).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Warn,
            &flock_args(),
        )
        .await
    };

    assert_eq!(code, ExitCode::Success);
    let table = String::from_utf8(out).unwrap();
    assert!(table.contains("unrecognized"), "{table}");
}

#[tokio::test]
async fn flock_json_against_a_skewed_shepherd_keeps_stdout_one_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let (paths, _requests) = skewed_shepherd(dir.path(), ProcStatus::Online).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        streams.fmt = Format::Json;
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Warn,
            &flock_args(),
        )
        .await
    };

    assert_eq!(code, ExitCode::Success);
    let stdout = String::from_utf8(out).unwrap();
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let envelope: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(envelope["command"], "flock");
    let stderr = String::from_utf8(err).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("\"notice\""), "{stderr}");
}

/// `describe` and `bleats` take their client from [`connect_client`] with
/// the guard [`VersionGuard::for_command`] chose, so the split reaches them
/// without a call-site change.
#[tokio::test]
async fn describe_and_bleats_go_through_connect_client_with_a_warn_guard() {
    for argv in [&["describe", "web"][..], &["bleats", "web"]] {
        let dir = tempfile::tempdir().unwrap();
        let (paths, _requests) = skewed_shepherd(dir.path(), ProcStatus::Online).await;
        let guard = VersionGuard::for_command(&verb(argv));
        assert_eq!(guard, VersionGuard::Warn, "{argv:?}");

        let mut out = Vec::new();
        let mut err = Vec::new();
        let connected = {
            let mut streams = buffered_streams(&mut out, &mut err);
            connect_client(&mut streams, &Link::Local(&paths), guard).await
        };

        assert!(connected.is_ok(), "{argv:?}");
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains(SKEWED_WARNING), "{argv:?}: {text}");
    }
}
