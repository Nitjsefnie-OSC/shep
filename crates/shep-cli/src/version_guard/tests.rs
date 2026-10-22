use std::io::IsTerminal;

use cli::{DaemonArgs, StartArgs};
use shep_core::paths::ShepPaths;
use shep_core::protocol::{Request, Response};
use shep_core::status::ProcStatus;

use super::*;
use crate::style;

/// A [`Streams`] over two byte buffers, so a refusal's exact text can be
/// read back. `BARE` because these tests assert on words, not colour.
fn buffered_streams<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>) -> Streams<'a> {
    Streams {
        out,
        err,
        style: style::Presentation::BARE,
        fmt: Format::Table,
    }
}

/// A real [`Client`], past a real handshake, whose peer announced
/// `version`. The [`shep_client::testing::FakeDaemon`] is returned so it
/// outlives the client.
async fn client_announcing(
    addr: &std::path::Path,
    version: &str,
) -> (Client, shep_client::testing::FakeDaemon) {
    let ack = shep_core::protocol::HelloAck {
        daemon_version: version.to_owned(),
        protocol: shep_core::protocol::PROTOCOL_VERSION,
        pid: 4242,
        min_supported: None,
    };
    shep_client::testing::fake_client_with_ack(addr, ack).await
}

/// The case a protocol-only check misses: the wire versions agree and the
/// crate versions do not.
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

/// Writes an executable `ssh` into `dir` that answers the handshake with
/// a shepherd of version 9.9.9 and then reads until the client hangs up.
#[cfg(unix)]
fn skewed_ssh_stand_in(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let ack = shep_core::protocol::HelloAck {
        daemon_version: "9.9.9".to_owned(),
        protocol: shep_core::protocol::PROTOCOL_VERSION,
        pid: 4242,
        min_supported: None,
    };
    let reply: shep_core::protocol::HelloReply = Ok(ack);
    let payload = shep_core::protocol::encode_frame(&reply).unwrap();
    let mut framed = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    framed.extend_from_slice(&payload);
    std::fs::write(dir.join("reply.bin"), framed).unwrap();
    let program = dir.join("ssh");
    std::fs::write(
        &program,
        "#!/bin/sh\ncat \"$(dirname \"$0\")/reply.bin\"\ncat >/dev/null\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    program
}

/// A remedy without `ssh <dest>` would reload this machine's shepherd,
/// which is not the one in skew.
#[cfg(unix)]
#[tokio::test]
async fn a_skew_over_a_host_names_the_remote_reload_command() {
    let dir = tempfile::tempdir().unwrap();
    let link = Link::Host {
        dest: "amp2",
        options: shep_client::SshOptions::default().program(skewed_ssh_stand_in(dir.path())),
    };

    for fmt in [Format::Table, Format::Json] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = {
            let mut streams = buffered_streams(&mut out, &mut err);
            streams.fmt = fmt;
            flock_command(&mut streams, &link, VersionGuard::Enforce, &flock_args()).await
        };

        assert_eq!(code, ExitCode::VersionSkew, "{fmt:?}");
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains("the shepherd on amp2 is 9.9.9"), "{text}");
        assert!(text.contains("ssh amp2 shep daemon reload"), "{text}");
        assert!(
            !text.contains("`cargo install shep`"),
            "the local cause does not explain a remote skew: {text}"
        );
        assert_eq!(
            text.matches("shep daemon reload").count(),
            1,
            "the remedy is named once: {text}"
        );
        assert!(
            !text.contains("Run `shep daemon reload`") && !text.contains("Run:\n    shep daemon"),
            "the bare command would reload this machine's shepherd: {text}"
        );
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
    }
}

/// The roll is this machine's `flock.json`. A dead host that printed it
/// would list the wrong flock under the right exit code.
#[tokio::test]
async fn flock_over_a_host_never_falls_back_to_the_local_roll() {
    let dir = tempfile::tempdir().unwrap();
    let paths = ShepPaths::resolve(&|_| None, dir.path());
    std::fs::create_dir_all(&paths.run).unwrap();
    let roll =
        shep_daemon::snapshot::FlockSnapshot::with_apps(vec![shep_daemon::snapshot::SavedApp {
            app: shep_core::config::AppConfig::minimal("local-sheep", "./srv"),
            instances_running: 1,
            disabled: false,
        }]);
    std::fs::write(&paths.snapshot, serde_json::to_vec(&roll).unwrap()).unwrap();
    let link = Link::Host {
        dest: "amp2",
        options: shep_client::SshOptions::default().program(dir.path().join("no-such-ssh")),
    };

    for fmt in [Format::Table, Format::Json] {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = {
            let mut streams = buffered_streams(&mut out, &mut err);
            streams.fmt = fmt;
            flock_command(&mut streams, &link, VersionGuard::Enforce, &flock_args()).await
        };

        assert_eq!(code, ExitCode::DaemonUnreachable, "{fmt:?}");
        assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
        let text = String::from_utf8(err).unwrap();
        assert!(!text.contains("local-sheep"), "{text}");
        assert!(!text.contains("saved roll"), "{text}");
    }
}

/// `--follow` reaches a host, so what stops it here is the terminal. The
/// host's own refusal text would send the operator after the wrong fix.
#[tokio::test]
async fn a_host_follow_without_a_terminal_refuses_for_the_terminal_not_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let link = Link::Host {
        dest: "amp2",
        options: shep_client::SshOptions::default().program(dir.path().join("no-such-ssh")),
    };
    let args = cli::FlockArgs {
        follow: true,
        interval: 1,
    };

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = buffered_streams(&mut out, &mut err);
        flock_command_on(&mut streams, &link, VersionGuard::Enforce, &args, false).await
    };

    assert_eq!(code, ExitCode::Usage);
    let text = String::from_utf8(err).unwrap();
    assert!(text.contains("needs a terminal"), "{text}");
    assert!(!text.contains(crate::remote::HOST_REFUSAL), "{text}");
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
    assert_eq!(version_skew_warning(&client, None), None);
}

#[tokio::test]
async fn the_warning_for_a_host_names_that_host_and_an_ssh_remedy() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8").await;

    assert_eq!(
        version_skew_warning(&client, Some("amp2")).as_deref(),
        Some(concat!(
            "this shep is ",
            env!("CARGO_PKG_VERSION"),
            ", the shepherd on amp2 is 0.1.8. Listing works, but verbs that change the flock \
                 refuse until it is reloaded. Run `ssh amp2 shep daemon reload`."
        ))
    );
}

/// The version arrives over the socket, so a control character in it must
/// not reach the terminal.
#[tokio::test]
async fn a_version_with_an_escape_in_it_is_collapsed_in_the_warning() {
    let dir = tempfile::tempdir().unwrap();
    let addr = shep_client::testing::control_address(dir.path());
    let (client, _fake) = client_announcing(&addr, "0.1.8\u{1b}[31m").await;

    let text = version_skew_warning(&client, None).unwrap();
    assert!(!text.contains('\u{1b}'), "{text:?}");
}

/// A home whose socket a fake shepherd on version `0.1.8` serves, listing
/// one sheep with `status`, and the channel of what it was sent.
async fn skewed_shepherd(
    dir: &std::path::Path,
    status: ProcStatus,
) -> (
    ShepPaths,
    tokio::sync::mpsc::UnboundedReceiver<shep_core::protocol::Envelope>,
) {
    let paths = ShepPaths::resolve(&|_| None, dir);
    std::fs::create_dir_all(&paths.run).unwrap();
    let mut info = shep_client::testing::sample_info();
    info.status = status;
    let ack = shep_core::protocol::HelloAck {
        daemon_version: "0.1.8".to_owned(),
        protocol: shep_core::protocol::PROTOCOL_VERSION,
        pid: 4242,
        min_supported: None,
    };
    let requests =
        shep_client::testing::fake_daemon_answering_with_ack(&paths.socket, ack, move |request| {
            match request {
                Request::ListFlock => Response::Flock(vec![info.clone()]),
                _ => Response::Pong,
            }
        })
        .await;
    // Held by the caller: the fake stops serving once nobody reads it.
    (paths, requests)
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

/// An empty [`StartArgs`], for asking which guard `add` gets. The guard
/// reads the verb alone, so no field here reaches it.
fn add_args() -> StartArgs {
    StartArgs {
        targets: Vec::new(),
        name: None,
        fold: None,
        cwd: None,
        interpreter: None,
        flockfile: false,
        no_log_timestamps: false,
        reset: None,
    }
}

/// A [`cli::FlockArgs`] that does not follow: the bare `shep flock`.
fn flock_args() -> cli::FlockArgs {
    cli::FlockArgs {
        follow: false,
        interval: cli::FOLLOW_INTERVAL_FLOOR_SECONDS,
    }
}

/// A [`DaemonArgs`] carrying `cmd` and nothing else, for asking which
/// guard the `daemon` verb's two shapes get.
fn daemon_args(cmd: Option<cli::DaemonCmd>) -> DaemonArgs {
    DaemonArgs {
        cmd,
        no_restore: false,
        foreground: false,
        log_json: None,
        log_level: None,
        socket: None,
        max_cron_sleep: None,
    }
}

/// Fails if a verb leaves [`RECOVERY_VERBS`], or arrives without being
/// listed there with its reason.
#[test]
fn every_exempt_verb_is_one_of_the_documented_recovery_verbs() {
    for command in [
        Commands::Kill,
        Commands::Ping,
        Commands::Daemon(daemon_args(Some(cli::DaemonCmd::Reload))),
    ] {
        let verb =
            recovery_verb(&command).unwrap_or_else(|| panic!("{command:?} must stay exempt"));
        assert!(
            RECOVERY_VERBS.contains(&verb),
            "{verb} is exempt but undocumented"
        );
    }
    assert_eq!(
        recovery_verb(&Commands::Daemon(daemon_args(Some(cli::DaemonCmd::Reload)))),
        Some("daemon reload"),
        "the verb the skew refusal names must be the verb the skew guard exempts"
    );
    // The hidden boot re-exec, which reaches no shepherd at all.
    assert_eq!(recovery_verb(&Commands::Daemon(daemon_args(None))), None);
    assert_eq!(
        VersionGuard::for_command(&Commands::Daemon(daemon_args(None))),
        VersionGuard::Enforce
    );
    assert_eq!(recovery_verb(&Commands::Flock(flock_args())), None);
    assert_eq!(
        VersionGuard::for_command(&Commands::Flock(flock_args())),
        VersionGuard::Warn
    );
    assert_eq!(
        VersionGuard::for_command(&Commands::Kill),
        VersionGuard::Exempt
    );
    // `add` carries a request an older shepherd cannot decode, and it
    // reaches `Enforce` through the `_` arm rather than by being named.
    assert_eq!(
        VersionGuard::for_command(&Commands::Add(add_args())),
        VersionGuard::Enforce
    );
}

/// The verb `argv` names, parsed the way the binary parses it.
fn verb(argv: &[&str]) -> Commands {
    use clap::Parser as _;

    let argv = std::iter::once("shep").chain(argv.iter().copied());
    cli::Cli::try_parse_from(argv).unwrap().command
}

/// Fails if a verb leaves [`READ_VERBS`], or arrives without being listed
/// there.
#[test]
fn every_read_verb_is_one_of_the_documented_read_verbs() {
    for (argv, name) in [
        (&["flock"][..], "flock"),
        (&["describe", "web"], "describe"),
        (&["bleats", "web"], "bleats"),
        (&["bleats", "web", "--no-follow"], "bleats"),
    ] {
        let command = verb(argv);
        let found = read_verb(&command).unwrap_or_else(|| panic!("{command:?} must stay a read"));
        assert_eq!(found, name);
        assert!(
            READ_VERBS.contains(&found),
            "{found} warns but is undocumented"
        );
        assert_eq!(VersionGuard::for_command(&command), VersionGuard::Warn);
    }
    assert_eq!(
        read_verb(&Commands::Kill),
        None,
        "a recovery verb is not a read verb"
    );
}

/// A verb that writes must not be widened into a warning by accident: each
/// of these changes the flock, or reaches a surface that can.
#[test]
fn every_verb_that_changes_the_flock_still_enforces() {
    for argv in [
        &["start", "web"][..],
        &["add", "web"],
        &["stop", "web"],
        &["restart", "web"],
        &["reload", "web"],
        &["delete", "web"],
        &["stock", "web", "2"],
        &["enable", "web"],
        &["disable", "web"],
        &["trigger", "web", "go"],
        &["signal", "web", "TERM"],
        &["whisper", "web", "hi"],
        &["reopen", "web"],
        &["flush", "web"],
        &["save"],
        &["muster"],
        &["copy", "web", "web2"],
        &["set", "key", "value"],
    ] {
        let command = verb(argv);
        assert_eq!(
            VersionGuard::for_command(&command),
            VersionGuard::Enforce,
            "{argv:?}"
        );
    }
}
