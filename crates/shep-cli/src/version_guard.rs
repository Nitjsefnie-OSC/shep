//! Protocol version safety: refusing a shepherd whose crate version differs
//! from this binary's, and `shep flock`'s own dispatch, which applies the
//! guard by hand around its roll fallback.

use std::io::IsTerminal;
use std::time::Duration;

use cli::{Commands, Format};
use commands::query;
use exit::ExitCode;
use output::Streams;
use shep_client::Client;

use crate::client::unreachable_message;
use crate::remote::{HOST_REFUSAL, Link};
use crate::{cli, commands, exit, output};

/// The verbs a version skew must never refuse, spelled the way an operator
/// types them.
///
/// A verb belongs here only if it is a way out of a skew, never merely one
/// that is inconvenient to lose: a guard whose remedy is itself guarded
/// leaves a live daemon and a live flock nothing can touch.
/// [`VERSION_SKEW_REMEDY`] reads `shep daemon reload` out of this list rather
/// than spelling it twice.
const RECOVERY_VERBS: [&str; 3] = ["kill", "daemon reload", "ping"];

/// Which [`RECOVERY_VERBS`] entry `command` is, or `None` for an ordinary
/// verb the version guard applies to.
///
/// Returns the name rather than a bool so a test can hold this mapping and
/// that list against each other.
fn recovery_verb(command: &Commands) -> Option<&'static str> {
    match command {
        Commands::Kill => Some("kill"),
        Commands::Ping => Some("ping"),
        // A bare `shep daemon` is the hidden boot re-exec, which reaches no
        // shepherd and so has nothing to be exempt from.
        Commands::Daemon(args) => match args.cmd {
            Some(cli::DaemonCmd::Reload) => Some("daemon reload"),
            None => None,
        },
        _ => None,
    }
}

/// Whether a shepherd of a different version refuses this invocation.
///
/// `pub(crate)`: `lookout`, `whistle` and `foreground` call `Client::connect`
/// in their own modules and name this at each connect site, always
/// [`Self::Enforce`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VersionGuard {
    /// Refuse: this verb needs a shepherd that agrees with this binary.
    Enforce,
    /// Never refuse, whatever the shepherd answers: one of
    /// [`RECOVERY_VERBS`].
    Exempt,
}

impl VersionGuard {
    /// The guard that applies to `command`.
    pub(crate) fn for_command(command: &Commands) -> Self {
        match recovery_verb(command) {
            Some(_) => Self::Exempt,
            None => Self::Enforce,
        }
    }
}

/// Why a skew happens, as the two lines the table form prints.
///
/// Held as rendered lines rather than one string: the JSON form joins them
/// with a space and the table form with a newline.
const VERSION_SKEW_CAUSE: [&str; 2] = [
    "`cargo install shep` replaced the binary. It did not restart the",
    "shepherd, which is still running the old code.",
];

/// Why a skew happens when the shepherd is on another host, as the two lines
/// the table form prints.
///
/// Not [`VERSION_SKEW_CAUSE`]: `cargo install` on this machine says nothing
/// about a shepherd that lives elsewhere.
const VERSION_SKEW_HOST_CAUSE: [&str; 2] = [
    "The shepherd on that host runs a different shep than this one.",
    "A reload runs the shep installed there; if they still differ, install the same version on both.",
];

/// The one command that fixes a skew.
///
/// Read out of [`RECOVERY_VERBS`] because the two have to agree: a remedy the
/// guard itself refused would be a dead end.
pub(crate) const VERSION_SKEW_REMEDY: &str = RECOVERY_VERBS[1];

/// The command that fixes a skew, spelled for where the shepherd is.
///
/// With a `host` it runs over ssh: the bare [`VERSION_SKEW_REMEDY`] would
/// reload this machine's shepherd, which is not the one in skew.
fn version_skew_command(host: Option<&str>) -> String {
    match host {
        Some(dest) => format!("ssh {dest} shep {VERSION_SKEW_REMEDY}"),
        None => format!("shep {VERSION_SKEW_REMEDY}"),
    }
}

/// The imperative naming [`version_skew_command`], in the two shapes the
/// formats need.
///
/// A `--format json` consumer gets a single-line message with the command
/// inside it. The table form has layout, and its label carries no blank line
/// after it: it sits directly on the command so the two read as one thing.
fn version_skew_instruction(fmt: Format, host: Option<&str>) -> String {
    let command = version_skew_command(host);
    match fmt {
        Format::Json => format!("Run `{command}`."),
        // Two spaces of its own, on top of the indent `safe_message` gives
        // every continuation line, so the command sits a level under its
        // label rather than beside it.
        Format::Table => format!("Run:\n  {command}"),
    }
}

/// Refuses a shepherd whose crate version differs from this binary's.
///
/// Any difference, not only a protocol difference: `cargo install shep`
/// replaces the binary and leaves the shepherd running the old code, so the
/// two can agree on every byte of the wire while disagreeing about what a
/// verb does. Compares [`shep_core::protocol::HelloAck::daemon_version`]
/// against `CARGO_PKG_VERSION`, read from a handshake that already succeeded.
///
/// # Errors
/// [`ExitCode::VersionSkew`], after writing the refusal to `streams`, when
/// `guard` is [`VersionGuard::Enforce`] and the shepherd reports a different
/// version. A [`VersionGuard::Exempt`] verb is always `Ok`.
///
/// A client reached over ssh gets a refusal that names its host and a remedy
/// that runs there.
pub(crate) fn refuse_version_skew(
    streams: &mut Streams<'_>,
    client: &Client,
    guard: VersionGuard,
) -> Result<(), ExitCode> {
    let running = client.daemon().daemon_version.as_str();
    if guard == VersionGuard::Exempt || running == env!("CARGO_PKG_VERSION") {
        return Ok(());
    }
    let code = ExitCode::VersionSkew;
    let host = client.host();
    let cause = match host {
        Some(_) => VERSION_SKEW_HOST_CAUSE,
        None => VERSION_SKEW_CAUSE,
    };
    let shepherd = match host {
        Some(dest) => format!("the shepherd on {dest}"),
        None => "the running shepherd".to_owned(),
    };
    let summary = format!(
        "this shep is {}, {shepherd} is {running}",
        env!("CARGO_PKG_VERSION")
    );
    match streams.fmt {
        // One line, one envelope. A `--format json` consumer has no use for
        // the layout below, and it still gets every fact in `error.message`.
        Format::Json => {
            let cause = cause.join(" ");
            let instruction = version_skew_instruction(Format::Json, host);
            streams.fail(code, &format!("{summary}. {cause} {instruction}"));
        }
        // The remedy has to sit on a line of its own to be copied.
        Format::Table => {
            let cause = cause.join("\n");
            // `daemon_version` arrives over the socket, and the table
            // emitter keeps line breaks, so this collapses the one fragment
            // a peer worded before it joins prose that does not.
            let summary = crate::terminal_safe::sanitise(&summary).0;
            let instruction = version_skew_instruction(Format::Table, host);
            streams.fail(code, &format!("{summary}\n\n{cause}\n\n{instruction}"));
        }
    }
    Err(code)
}

/// Connects to the shepherd `link` names: the socket at `paths.socket`, or a
/// host over ssh. Never autostarts.
///
/// The one seam every verb needing a [`Client`] passes through, so a shepherd
/// of a different version is refused here and no verb has to remember to ask.
pub(crate) async fn connect_client(
    streams: &mut Streams<'_>,
    link: &Link<'_>,
    guard: VersionGuard,
) -> Result<Client, ExitCode> {
    let dialled = match link {
        Link::Local(paths) => Client::connect(&paths.socket).await,
        Link::Host { dest, options } => Client::connect_ssh_with(dest, options).await,
    };
    match dialled {
        Ok(client) => {
            refuse_version_skew(streams, &client, guard)?;
            Ok(client)
        }
        Err(err) => {
            let code = ExitCode::from(&err);
            Err(streams.fail(code, &unreachable_message(&err)))
        }
    }
}

/// `shep flock`'s own dispatch, split out of [`run`](crate::dispatch::run) so
/// a test can drive it against a real fixture socket.
///
/// Uses its own `Client::connect`, not [`connect_client`], because that
/// helper reports and gives up and this arm has a roll fallback to reach.
/// The version guard is applied by hand here.
///
/// A refusal is not an absence. The roll fallback is for
/// [`shep_client::ConnectError::Connect`] alone, nothing listening at all;
/// every other variant means the shepherd is there and answered.
///
/// Over a host there is no roll fallback: the roll is this machine's
/// `flock.json`, so printing it for a dead host would list the wrong flock.
/// The connect error's own code ends the run instead.
pub(crate) async fn flock_command(
    streams: &mut Streams<'_>,
    link: &Link<'_>,
    guard: VersionGuard,
    args: &cli::FlockArgs,
) -> ExitCode {
    let paths = match link {
        Link::Local(paths) => *paths,
        // Lifted by the slice that gives `--follow` a host.
        Link::Host { .. } if args.follow => return streams.fail(ExitCode::Usage, HOST_REFUSAL),
        Link::Host { .. } => {
            return match connect_client(streams, link, guard).await {
                Ok(client) => query::flock(&client, streams).await,
                Err(code) => code,
            };
        }
    };
    if args.follow {
        return follow_flock_command(streams, link, guard, args).await;
    }
    match Client::connect(&paths.socket).await {
        Ok(client) => match refuse_version_skew(streams, &client, guard) {
            Ok(()) => query::flock(&client, streams).await,
            // A skew is not an absence either: the roll fallback below would
            // print a listing while hiding why every other verb is refusing.
            Err(code) => code,
        },
        // The roll fallback is a table-format affordance: under `--format
        // json` a failed invocation leaves stdout empty and puts an error
        // envelope on stderr, absence included.
        Err(err) if streams.fmt == Format::Json => {
            let code = ExitCode::from(&err);
            streams.fail(code, &unreachable_message(&err))
        }
        Err(shep_client::ConnectError::Connect { .. }) => query::flock_from_roll(streams, paths),
        Err(err) => {
            let code = ExitCode::from(&err);
            streams.fail(code, &flock_connect_refusal_message(&err))
        }
    }
}

/// `shep flock --follow`'s own dispatch: two refusals, then
/// [`connect_client`] and the redraw loop.
///
/// [`connect_client`], not [`flock_command`]'s hand-rolled connect, because
/// this path has no roll to fall back to. A saved roll is one moment, and a
/// follow exists for the moments after it, so a shepherd that is not running
/// is a refusal here rather than a listing.
///
/// Both refusals are usage errors rather than degradations. A follow written
/// into a file would be a file of escape sequences, and a follow that quietly
/// printed once instead would exit zero having done something else than what
/// was asked.
async fn follow_flock_command(
    streams: &mut Streams<'_>,
    link: &Link<'_>,
    guard: VersionGuard,
    args: &cli::FlockArgs,
) -> ExitCode {
    if streams.fmt == Format::Json {
        return streams.fail(
            ExitCode::Usage,
            "`--follow` redraws a table; `--format json` has no follow form",
        );
    }
    if !std::io::stdout().is_terminal() {
        return streams.fail(
            ExitCode::Usage,
            "`--follow` needs a terminal; stdout is not one",
        );
    }
    match connect_client(streams, link, guard).await {
        Ok(client) => {
            query::flock_follow(&client, streams, Duration::from_secs(args.interval)).await
        }
        Err(code) => code,
    }
}

/// Renders `err` for [`flock_command`]'s refusal arm: the shepherd is there,
/// so this reports what it did and names the fix rather than the roll's "no
/// shepherd running".
fn flock_connect_refusal_message(err: &shep_client::ConnectError) -> String {
    format!("{err}; run `shep {VERSION_SKEW_REMEDY}`")
}

#[cfg(test)]
mod tests {
    use std::io::IsTerminal;

    use cli::{DaemonArgs, StartArgs};
    use shep_core::paths::ShepPaths;

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
            flock_command(&mut streams, &Link::Local(&paths), VersionGuard::Enforce, &flock_args()).await
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
            flock_command(&mut streams, &Link::Local(&paths), VersionGuard::Enforce, &flock_args()).await
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
            flock_command(&mut streams, &Link::Local(&paths), VersionGuard::Enforce, &flock_args()).await
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
            flock_command(&mut streams, &Link::Local(&paths), VersionGuard::Enforce, &args).await
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
            flock_command(&mut streams, &Link::Local(&paths), VersionGuard::Enforce, &args).await
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
        let roll = shep_daemon::snapshot::FlockSnapshot::with_apps(vec![
            shep_daemon::snapshot::SavedApp {
                app: shep_core::config::AppConfig::minimal("local-sheep", "./srv"),
                instances_running: 1,
                disabled: false,
            },
        ]);
        std::fs::write(&paths.snapshot, serde_json::to_vec(&roll).unwrap()).unwrap();
        let link = Link::Host {
            dest: "amp2",
            options: shep_client::SshOptions::default()
                .program(dir.path().join("no-such-ssh")),
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

    /// `--follow` over a host waits for the slice that gives it one, and
    /// refuses with the one message every verb that cannot reach a host gets.
    #[tokio::test]
    async fn follow_over_a_host_is_refused_with_the_one_host_message() {
        let dir = tempfile::tempdir().unwrap();
        let link = Link::Host {
            dest: "amp2",
            options: shep_client::SshOptions::default()
                .program(dir.path().join("no-such-ssh")),
        };
        let args = cli::FlockArgs {
            follow: true,
            interval: 1,
        };

        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = {
            let mut streams = buffered_streams(&mut out, &mut err);
            flock_command(&mut streams, &link, VersionGuard::Enforce, &args).await
        };

        assert_eq!(code, ExitCode::Usage);
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains(HOST_REFUSAL), "{text}");
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
            VersionGuard::Enforce
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
}
