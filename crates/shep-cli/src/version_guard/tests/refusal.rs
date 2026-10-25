//! The local socket's half: a crate-version difference refuses a verb that
//! writes, `flock` still falls back to the roll for a genuine absence, and
//! the recovery verbs are never refused.

use super::*;

/// The case a protocol-only check misses: the wire versions agree and the
/// crate versions do not.
///
/// Local only, which is the half of the guard that compares crate versions:
/// over `--host` the same handshake is a notice, and
/// `host::a_host_on_another_version_but_the_same_protocol_is_written_to_and_warns`
/// holds that side.
#[tokio::test]
async fn a_version_difference_with_no_protocol_difference_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut streams = buffered_streams(&mut out, &mut err);
    let code = refuse_version_skew(&mut streams, &client, VersionGuard::Enforce)
        .expect_err("a differing crate version must be refused");
    assert_eq!(code, ExitCode::VersionSkew);
}

#[tokio::test]
async fn the_error_names_the_command_that_fixes_it() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        let _ = refuse_version_skew(&mut streams, &client, VersionGuard::Enforce);
    }
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains("error[version_skew]"), "{text}");
    assert!(text.contains("this shep is"), "{text}");
    assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    assert!(text.contains("the running shepherd is 0.1.8"), "{text}");
    assert!(
        text.contains("`cargo install shep` replaced the binary"),
        "{text}"
    );
    assert!(text.contains("shep daemon reload"), "{text}");
}

/// The indentation is what makes the remedy line copyable, so it sits on
/// its own line rather than folded into the sentence above it.
#[tokio::test]
async fn the_table_form_names_the_remedy_as_an_instruction_not_only_a_line() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        let _ = refuse_version_skew(&mut streams, &client, VersionGuard::Enforce);
    }
    let text = String::from_utf8(err).unwrap();

    // No blank line between the label and the command: a gap reads as
    // two unrelated things. Four spaces, not two: `safe_message` indents
    // every continuation line and the instruction adds one level on top,
    // so the command sits under its label rather than beside it.
    assert!(
        text.contains("Run:\n    shep daemon reload"),
        "the label must sit directly on the copyable line it points at: {text}"
    );
    // The sentence above the indented line must not repeat the command.
    assert_eq!(
        text.matches("shep daemon reload").count(),
        1,
        "the remedy is named once, not restated in prose: {text}"
    );
}

/// A daemon that answered the handshake and refused it is not an absence:
/// "no shepherd running" would send the operator to the roll instead.
#[tokio::test]
async fn flock_reports_a_refusal_as_a_refusal_not_as_no_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    std::fs::create_dir_all(&paths.run).unwrap();
    let refusal = shep_core::protocol::RpcError {
        code: shep_core::protocol::RpcErrorCode::ProtocolMismatch,
        message: "this daemon speaks protocol 1, this client speaks 2".to_string(),
        daemon_version: Some("0.1.8".to_string()),
    };
    let _daemon = shep_client::testing::fake_daemon(&paths.socket, Err(refusal)).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Enforce,
            &flock_args(),
        )
        .await
    };

    assert_ne!(code, ExitCode::Success);
    let text = String::from_utf8(err).unwrap();
    assert!(
        !text.contains("no shepherd running"),
        "a refusal is not an absence: {text}"
    );
    assert!(text.contains("shep daemon reload"), "{text}");
}

/// `fake_daemon` serves one connection and then stops listening, so a
/// second connect would find nothing and report an absence in the
/// refusal's place.
#[tokio::test]
async fn flock_json_reports_the_refusal_it_got_not_a_second_connect_failure() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    std::fs::create_dir_all(&paths.run).unwrap();
    let refusal = shep_core::protocol::RpcError {
        code: shep_core::protocol::RpcErrorCode::ProtocolMismatch,
        message: "this daemon speaks protocol 1, this client speaks 2".to_string(),
        daemon_version: Some("0.1.8".to_string()),
    };
    let daemon = shep_client::testing::fake_daemon(&paths.socket, Err(refusal)).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        streams.fmt = Format::Json;
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Enforce,
            &flock_args(),
        )
        .await
    };
    daemon.await.unwrap();

    assert_eq!(code, ExitCode::ProtocolMismatch);
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains("this daemon speaks protocol 1"), "{text}");
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
}

#[tokio::test]
async fn flock_still_falls_back_to_the_roll_for_a_genuine_absence() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    std::fs::create_dir_all(&paths.run).unwrap();
    // No socket bound at `paths.socket`, so `connect(2)` itself fails.

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Enforce,
            &flock_args(),
        )
        .await
    };

    assert_eq!(code, ExitCode::DaemonUnreachable);
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains("no shepherd running"), "{text}");
}

/// Both of `--follow`'s refusals carry `ExitCode::Usage`, so the code
/// alone cannot tell them apart: delete either guard and a test asserting
/// only the code still passes. Each is pinned by its own message instead.
///
/// Neither arm needs a fixture. The `--format json` case proves the first
/// guard answered rather than the second, since a table-format follow
/// under a pipe would have refused too, with different words.
#[tokio::test]
async fn follow_refuses_json_and_a_pipe_for_different_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    let args = cli::FlockArgs {
        follow: true,
        interval: 1,
    };

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        streams.fmt = Format::Json;
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Enforce,
            &args,
        )
        .await
    };
    assert_eq!(code, ExitCode::Usage);
    let text = String::from_utf8(err).unwrap();
    assert!(
        text.contains("no follow form"),
        "the json guard answers before the terminal one: {text}"
    );

    // `cargo test` captures stdout, so the terminal guard fires on its
    // own. Under `--nocapture` on a real terminal it cannot, and there is
    // nothing to pin rather than something to fail.
    if std::io::stdout().is_terminal() {
        return;
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        flock_command(
            &mut streams,
            &Link::Local(&paths),
            VersionGuard::Enforce,
            &args,
        )
        .await
    };
    assert_eq!(code, ExitCode::Usage);
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains("needs a terminal"), "{text}");
    assert!(
        out.is_empty(),
        "a refused follow prints no listing: {}",
        String::from_utf8_lossy(&out)
    );
}

#[tokio::test]
async fn a_matching_version_passes_without_a_word() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, env!("CARGO_PKG_VERSION")).await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        refuse_version_skew(&mut streams, &client, VersionGuard::Enforce)
            .expect("a matching version is not a skew");
    }
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
}

#[tokio::test]
async fn the_recovery_verbs_are_not_refused() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    {
        let mut streams = buffered_streams(&mut out, &mut err);
        refuse_version_skew(&mut streams, &client, VersionGuard::Exempt)
            .expect("a recovery verb is never refused on version skew");
    }
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
}
