//! `shep flock` with a `hosts` list: every shepherd's flock in one run, this
//! machine's first and then each listed host's in list order.
//!
//! The hosts are asked at once and nothing is printed until all of them have
//! answered, so the order of the output is the list's and never the
//! network's. A host that fails prints one line on stderr instead of a table
//! and does not hide the others.

use std::time::Duration;

use futures_util::future::join_all;
use shep_client::{Client, ConnectError, RequestError, SshOptions};
use shep_core::destination::Destination;
use shep_core::paths::ShepPaths;
use shep_core::protocol::{HostUsage, ProcessInfo};

use super::flock_display::{flock_from_roll, host_usage, print_listing};
use crate::cli::Format;
use crate::client::unreachable_message;
use crate::commands::hosts::{HOST_CONNECT_TIMEOUT, unattended};
use crate::commands::rpc::UNRECOGNISED;
use crate::commands::shep_toml::ShepToml;
use crate::exit::ExitCode;
use crate::output::{Streams, write_outcome};
use crate::version_guard::{
    VersionGuard, flock_connect_refusal_message, refuse_version_skew, version_skew_warning,
};

/// What the table form heads this machine's listing with.
const LOCAL_HEADING: &str = "local";

/// One shepherd's answer to a listing.
struct Answer {
    procs: Vec<ProcessInfo>,
    usage: Option<Option<HostUsage>>,
}

/// A host's answer, and the version warning it earned.
struct HostAnswer {
    answer: Answer,
    skew: Option<String>,
}

/// Why one shepherd gave no listing.
struct Failure {
    code: ExitCode,
    message: String,
}

impl Failure {
    fn of_connect(err: &ConnectError) -> Self {
        Self {
            code: ExitCode::from(err),
            message: unreachable_message(err),
        }
    }

    fn of_request(err: &RequestError) -> Self {
        match err {
            RequestError::UnexpectedReply { .. } => Self {
                code: ExitCode::Internal,
                message: UNRECOGNISED.to_owned(),
            },
            other => Self {
                code: ExitCode::from(other),
                message: other.to_string(),
            },
        }
    }
}

/// The hosts `paths`' `shep.toml` lists, or none.
///
/// A file that will not read or parse is none too, with one aside saying so:
/// a broken list must not stop a local `shep flock`. The error's own text is
/// left out because a parse error quotes the line it stopped on; `shep host
/// list` reports it in full.
pub(crate) fn listed_hosts(streams: &mut Streams<'_>, paths: &ShepPaths) -> Vec<String> {
    let names = ShepToml::read_only(&paths.daemon_config).and_then(|doc| doc.host_names());
    names.unwrap_or_else(|_| {
        let message = format!(
            "the hosts list in {} was not read, so only this machine's flock is listed; \
             `shep host list` says why",
            paths.daemon_config.display()
        );
        streams.aside("hosts_unread", &message);
        Vec::new()
    })
}

/// `shep flock` over `hosts`, dialled without prompts under the bound every
/// unattended dial shares.
///
/// # Errors
/// Returns [`ExitCode::Usage`] under `--format json`, which has no fleet form
/// yet. Otherwise as [`fleet`].
pub(crate) async fn flock_all(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    guard: VersionGuard,
    hosts: &[String],
) -> ExitCode {
    if streams.fmt == Format::Json {
        return streams.fail(
            ExitCode::Usage,
            "a hosts list is set, and `--format json` has no listing of every host yet; \
             `--host <name>` lists one host as JSON",
        );
    }
    let ssh = unattended(HOST_CONNECT_TIMEOUT);
    fleet(streams, paths, guard, hosts, &ssh, HOST_CONNECT_TIMEOUT).await
}

/// This machine's flock and then each of `hosts`, one table apiece.
///
/// Every host is dialled through `ssh` and given `timeout` for the dial and
/// both requests together: a box that accepts the connection and then says
/// nothing is abandoned, and the others still print. Returns
/// [`ExitCode::Success`] only when every shepherd answered, otherwise the
/// code of the first one that did not, this machine's first.
pub(super) async fn fleet(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    guard: VersionGuard,
    hosts: &[String],
    ssh: &SshOptions,
    timeout: Duration,
) -> ExitCode {
    let (local, remote) = tokio::join!(
        Client::connect(&paths.socket),
        join_all(hosts.iter().map(|host| ask_host(host, ssh, timeout))),
    );

    let mut tables = Tables::default();
    let mut first_failure = None;
    let mut note = |code: ExitCode| {
        first_failure.get_or_insert(code);
    };

    match ask_local(streams, guard, local).await {
        Local::Listed(answer) => {
            if let Err(done) = tables.print(streams, LOCAL_HEADING, answer, None) {
                return done;
            }
        }
        Local::Roll => {
            note(print_roll(streams, &mut tables, paths));
        }
        Local::Failed(code) => note(code),
    }
    for (host, result) in hosts.iter().zip(remote) {
        match result {
            Ok(HostAnswer { answer, skew }) => {
                if let Err(done) = tables.print(streams, host, answer, skew) {
                    return done;
                }
            }
            Err(Failure { code, message }) => {
                note(streams.fail(code, &format!("{host}: {message}")));
            }
        }
    }
    first_failure.unwrap_or(ExitCode::Success)
}

/// Whether a table has been printed yet, so the next one is set apart from it.
#[derive(Default)]
struct Tables {
    printed: bool,
}

impl Tables {
    /// Writes the blank line that separates this table from the last one, and
    /// the heading that names whose it is.
    fn open(&mut self, streams: &mut Streams<'_>, heading: &str) {
        if self.printed {
            let _ = writeln!(streams.out);
        }
        self.printed = true;
        let _ = writeln!(streams.out, "{heading}");
    }

    /// Prints `answer` under `heading`, after the warning `skew` carries.
    ///
    /// # Errors
    /// The exit code to end the run with when stdout is closed.
    fn print(
        &mut self,
        streams: &mut Streams<'_>,
        heading: &str,
        answer: Answer,
        skew: Option<String>,
    ) -> Result<(), ExitCode> {
        if let Some(warning) = skew {
            streams.aside("version_skew", &warning);
        }
        self.open(streams, heading);
        match print_listing(streams, answer.procs, answer.usage) {
            Ok(()) => Ok(()),
            Err(err) => Err(write_outcome(Err(err))),
        }
    }
}

/// How this machine's shepherd answered.
enum Local {
    Listed(Answer),
    /// Nothing is listening; [`print_roll`] speaks for it.
    Roll,
    /// Already reported on stderr.
    Failed(ExitCode),
}

/// Reads this machine's listing off `connected`, applying `guard` as
/// `shep flock` does.
async fn ask_local(
    streams: &mut Streams<'_>,
    guard: VersionGuard,
    connected: Result<Client, ConnectError>,
) -> Local {
    let client = match connected {
        Ok(client) => client,
        Err(ConnectError::Connect { .. }) => return Local::Roll,
        Err(err) => {
            let message = format!("{LOCAL_HEADING}: {}", flock_connect_refusal_message(&err));
            return Local::Failed(streams.fail(ExitCode::from(&err), &message));
        }
    };
    if let Err(code) = refuse_version_skew(streams, &client, guard) {
        return Local::Failed(code);
    }
    match fetch(&client).await {
        Ok(answer) => Local::Listed(answer),
        Err(Failure { code, message }) => {
            Local::Failed(streams.fail(code, &format!("{LOCAL_HEADING}: {message}")))
        }
    }
}

/// The saved roll of the shepherd that is not running, under the `local`
/// heading, and the code it counts as.
///
/// The roll is printed into a buffer first so an empty one, which prints
/// nothing, leaves no heading over nothing.
fn print_roll(streams: &mut Streams<'_>, tables: &mut Tables, paths: &ShepPaths) -> ExitCode {
    let mut rolled = Vec::new();
    let code = flock_from_roll(
        &mut Streams {
            out: &mut rolled,
            err: &mut *streams.err,
            style: streams.style,
            fmt: streams.fmt,
        },
        paths,
    );
    if !rolled.is_empty() {
        tables.open(streams, LOCAL_HEADING);
        let _ = streams.out.write_all(&rolled);
    }
    code
}

/// `Request::ListFlock` and then the host usage request, as `shep flock`
/// asks them.
async fn fetch(client: &Client) -> Result<Answer, Failure> {
    let procs = client
        .list_flock()
        .await
        .map_err(|e| Failure::of_request(&e))?;
    let usage = host_usage(client).await;
    Ok(Answer { procs, usage })
}

/// One host's listing, or why there is none.
///
/// `host` is parsed as a [`Destination`] before ssh sees it, so a hand-edited
/// entry that would read as an ssh option is that host's failure alone.
async fn ask_host(host: &str, ssh: &SshOptions, timeout: Duration) -> Result<HostAnswer, Failure> {
    let destination = Destination::parse(host).map_err(|err| Failure {
        code: ExitCode::InvalidConfig,
        message: format!("not a destination: {err}"),
    })?;
    let dialled = tokio::time::timeout(timeout, async {
        let client = Client::connect_ssh_with(destination.as_str(), ssh)
            .await
            .map_err(|e| Failure::of_connect(&e))?;
        let skew = version_skew_warning(&client, Some(destination.as_str()));
        let answer = fetch(&client).await;
        let _ = client.close().await;
        Ok(HostAnswer {
            answer: answer?,
            skew,
        })
    })
    .await;
    dialled.unwrap_or_else(|_elapsed| {
        Err(Failure {
            code: ExitCode::DaemonUnreachable,
            message: format!("did not answer within {timeout:?}"),
        })
    })
}

#[cfg(all(test, unix))]
mod tests;
