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
use crate::remote::Link;
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

/// The verbs that only read the flock, spelled the way an operator types
/// them, which a version skew warns about and does not refuse.
///
/// `bleats` is the whole verb: both its forms send one `ListFlock` and then
/// read. A verb belongs here only if it changes nothing, so a pane or a tool
/// that can write (`lookout`, `whistle`) stays out whatever it mostly does.
///
/// Read only by the test that holds it against [`read_verb`].
#[allow(dead_code)]
const READ_VERBS: [&str; 3] = ["flock", "describe", "bleats"];

/// Which [`READ_VERBS`] entry `command` is, or `None` for a verb that
/// changes the flock or is a recovery verb.
///
/// Returns the name rather than a bool so a test can hold this mapping and
/// that list against each other.
fn read_verb(command: &Commands) -> Option<&'static str> {
    match command {
        Commands::Flock(_) => Some("flock"),
        Commands::Describe(_) => Some("describe"),
        Commands::Bleats(_) => Some("bleats"),
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
    /// Proceed and say so on stderr: one of [`READ_VERBS`], which change
    /// nothing, so a shepherd on another version can still be listed.
    Warn,
}

impl VersionGuard {
    /// The guard that applies to `command`.
    pub(crate) fn for_command(command: &Commands) -> Self {
        match (recovery_verb(command), read_verb(command)) {
            (Some(_), _) => Self::Exempt,
            (None, Some(_)) => Self::Warn,
            (None, None) => Self::Enforce,
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

/// The opening clause of a skew message: both versions, and where the
/// shepherd runs.
fn version_skew_summary(running: &str, host: Option<&str>) -> String {
    let shepherd = match host {
        Some(dest) => format!("the shepherd on {dest}"),
        None => "the running shepherd".to_owned(),
    };
    format!(
        "this shep is {}, {shepherd} is {running}",
        env!("CARGO_PKG_VERSION")
    )
}

/// The one line a read verb prints when the shepherd runs another version,
/// or `None` when the versions match.
///
/// With a `host` the remedy runs over ssh, as the refusal's does.
pub(crate) fn version_skew_warning(client: &Client, host: Option<&str>) -> Option<String> {
    let running = client.daemon().daemon_version.as_str();
    if running == env!("CARGO_PKG_VERSION") {
        return None;
    }
    let summary = version_skew_summary(running, host);
    let command = version_skew_command(host);
    // `daemon_version` arrives over the socket, so the sentence is
    // collapsed as the table refusal's is.
    let sentence = format!(
        "{summary}. Listing works, but verbs that change the flock refuse until it is reloaded. Run `{command}`."
    );
    Some(crate::terminal_safe::sanitise(&sentence).0)
}

/// Refuses a shepherd whose crate version differs from this binary's, or
/// warns about it for a read verb.
///
/// Any difference, not only a protocol difference: `cargo install shep`
/// replaces the binary and leaves the shepherd running the old code, so the
/// two can agree on every byte of the wire while disagreeing about what a
/// verb does. Compares [`shep_core::protocol::HelloAck::daemon_version`]
/// against `CARGO_PKG_VERSION`, read from a handshake that already succeeded.
///
/// [`VersionGuard::Warn`] writes [`version_skew_warning`] to stderr as a
/// notice and proceeds, so stdout stays whatever the verb prints.
///
/// # Errors
/// [`ExitCode::VersionSkew`], after writing the refusal to `streams`, when
/// `guard` is [`VersionGuard::Enforce`] and the shepherd reports a different
/// version. A [`VersionGuard::Exempt`] or [`VersionGuard::Warn`] verb is
/// always `Ok`.
///
/// A client reached over ssh gets a refusal or a warning that names its host
/// and a remedy that runs there.
pub(crate) fn refuse_version_skew(
    streams: &mut Streams<'_>,
    client: &Client,
    guard: VersionGuard,
) -> Result<(), ExitCode> {
    let running = client.daemon().daemon_version.as_str();
    match guard {
        VersionGuard::Exempt => return Ok(()),
        VersionGuard::Warn => {
            if let Some(warning) = version_skew_warning(client, client.host()) {
                streams.aside("version_skew", &warning);
            }
            return Ok(());
        }
        VersionGuard::Enforce if running == env!("CARGO_PKG_VERSION") => return Ok(()),
        VersionGuard::Enforce => {}
    }
    let code = ExitCode::VersionSkew;
    let host = client.host();
    let cause = match host {
        Some(_) => VERSION_SKEW_HOST_CAUSE,
        None => VERSION_SKEW_CAUSE,
    };
    let summary = version_skew_summary(running, host);
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
/// With a `hosts` list in this machine's `shep.toml`, and neither `--host` nor
/// `--follow`, the arm is [`query::fleet::flock_all`] instead: every host's
/// flock, this machine's first.
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
    flock_command_on(streams, link, guard, args, std::io::stdout().is_terminal()).await
}

/// [`flock_command`] with whether stdout is a terminal decided by the caller,
/// so a test of the terminal refusal holds under `--nocapture` too.
pub(crate) async fn flock_command_on(
    streams: &mut Streams<'_>,
    link: &Link<'_>,
    guard: VersionGuard,
    args: &cli::FlockArgs,
    stdout_is_terminal: bool,
) -> ExitCode {
    let paths = match link {
        Link::Local(paths) => *paths,
        Link::Host { .. } if args.follow => {
            return follow_flock_command(streams, link, guard, args, stdout_is_terminal).await;
        }
        Link::Host { .. } => {
            return match connect_client(streams, link, guard).await {
                Ok(client) => query::flock(&client, streams).await,
                Err(code) => code,
            };
        }
    };
    if args.follow {
        return follow_flock_command(streams, link, guard, args, stdout_is_terminal).await;
    }
    let hosts = query::fleet::listed_hosts(streams, paths);
    if !hosts.is_empty() {
        return query::fleet::flock_all(streams, paths, guard, &hosts).await;
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
    stdout_is_terminal: bool,
) -> ExitCode {
    if streams.fmt == Format::Json {
        return streams.fail(
            ExitCode::Usage,
            "`--follow` redraws a table; `--format json` has no follow form",
        );
    }
    if !stdout_is_terminal {
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
pub(crate) fn flock_connect_refusal_message(err: &shep_client::ConnectError) -> String {
    format!("{err}; run `shep {VERSION_SKEW_REMEDY}`")
}

#[cfg(test)]
mod tests;
