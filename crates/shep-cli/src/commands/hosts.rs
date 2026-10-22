//! `shep host add` / `remove` / `list`: the CLI half of `shep.toml`'s
//! top-level `hosts` array.
//!
//! The verbs edit a list and a later slice reads it, so `remove` and `list`
//! never connect. `add` saves first and then dials the entry once through
//! [`Client::connect_ssh_with`], so a typo shows up while it can be fixed. The
//! attempt only warns: the entry stays and the exit is
//! [`ExitCode::Success`]. Every entry is checked against [`Destination`]
//! before this module hands it on, so a hand-edited `-oProxyCommand=...` is
//! named and refused rather than skipped or passed along.

use std::time::Duration;

use shep_client::{Client, ConnectError, SshCause, SshOptions};
use shep_core::destination::{Destination, DestinationError};
use shep_core::paths::ShepPaths;

use crate::cli::{Format, HostArgs, HostCommand};
use crate::commands::shep_toml::{ShepToml, ShepTomlError};
use crate::exit::ExitCode;
use crate::output::{HostEntry, HostRows, Streams, emit, write_outcome};

/// How long `host add` waits for a host to answer, and the bound a fleet
/// listing dials under: the entry is used without a terminal, so the check
/// must not wait on one.
pub(crate) const HOST_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The ssh settings every unattended dial of a listed host runs under: no
/// prompts, and bounded by `timeout`.
pub(crate) fn unattended(timeout: Duration) -> SshOptions {
    SshOptions::default()
        .batch_mode(true)
        .timeout(Some(timeout))
}

/// Why a `host` edit or read stopped.
enum HostError {
    /// `shep.toml` could not be read, parsed, or written.
    Config(ShepTomlError),
    /// An entry already in `shep.toml` is not a destination.
    BadEntry {
        entry: String,
        source: DestinationError,
    },
    /// `add` found the entry listed. Raised from inside the edit so the file
    /// is not rewritten byte for byte; carries the list as it stands.
    AlreadyListed(Vec<String>),
    /// `remove` found no such entry. Raised for the same reason.
    Absent,
}

impl From<ShepTomlError> for HostError {
    fn from(err: ShepTomlError) -> Self {
        Self::Config(err)
    }
}

/// `names`, each checked against the destination grammar.
///
/// A file an operator hand-edited can hold anything, and shep-core does not
/// look, so this is where a bad entry is stopped.
fn validated(names: Vec<String>) -> Result<Vec<String>, HostError> {
    for entry in &names {
        if let Err(source) = Destination::parse(entry) {
            return Err(HostError::BadEntry {
                entry: entry.clone(),
                source,
            });
        }
    }
    Ok(names)
}

/// Renders `err` and returns the code it maps to.
///
/// A file that is wrong is [`ExitCode::InvalidConfig`] and an unreadable one
/// is [`ExitCode::Failure`]. The other two variants never reach here: the
/// callers answer them themselves.
fn fail(streams: &mut Streams<'_>, err: &HostError) -> ExitCode {
    match err {
        HostError::Config(err) => crate::commands::dogs::fail_config(streams, err),
        HostError::BadEntry { entry, source } => {
            let message =
                format!("shep.toml: hosts entry {entry:?} is not a destination: {source}");
            streams.fail(ExitCode::InvalidConfig, &message)
        }
        HostError::AlreadyListed(_) | HostError::Absent => {
            streams.fail(ExitCode::Failure, "internal: unhandled host outcome")
        }
    }
}

/// Parses an `add` or `remove` argument, reporting a refusal as
/// [`ExitCode::Usage`].
fn parse_argument(streams: &mut Streams<'_>, text: &str) -> Result<Destination, ExitCode> {
    Destination::parse(text).map_err(|err| {
        let message = format!("{text:?} is not a destination: {err}");
        streams.fail(ExitCode::Usage, &message)
    })
}

/// Renders `names` as the one list `host` reports.
///
/// An empty list is a note in the table form and `[]` in JSON, so a script
/// parses the same shape whether or not anything is listed.
fn emit_hosts(streams: &mut Streams<'_>, names: Vec<String>) -> ExitCode {
    if names.is_empty() && streams.fmt == Format::Table {
        streams.note(
            "no_hosts",
            "no hosts. `shep host add <destination>` adds one.",
        );
        return ExitCode::Success;
    }
    let rows = HostRows(names.into_iter().map(|host| HostEntry { host }).collect());
    write_outcome(emit(
        &mut *streams.out,
        streams.fmt,
        "host",
        rows,
        streams.style,
    ))
}

/// `shep host <add|remove|list>`.
pub async fn host(streams: &mut Streams<'_>, paths: &ShepPaths, args: &HostArgs) -> ExitCode {
    host_with(streams, paths, args, &unattended(HOST_CONNECT_TIMEOUT)).await
}

/// [`host`] with the ssh settings `add` dials under. The seam for tests,
/// which name a stand-in program and a short timeout.
async fn host_with(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    args: &HostArgs,
    ssh: &SshOptions,
) -> ExitCode {
    match &args.command {
        HostCommand::Add { destination } => add(streams, paths, destination, ssh).await,
        HostCommand::Remove { destination } => remove(streams, paths, destination),
        HostCommand::List => list(streams, paths),
    }
}

/// `shep host add <destination>`.
///
/// Idempotent: a destination already listed exits [`ExitCode::Success`],
/// prints the list, and says so on stderr. Either way the destination is then
/// dialled once; see [`probe`].
async fn add(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    text: &str,
    ssh: &SshOptions,
) -> ExitCode {
    let destination = match parse_argument(streams, text) {
        Ok(destination) => destination,
        Err(code) => return code,
    };
    let edited = ShepToml::try_edit(&paths.daemon_config, |doc| {
        validated(doc.host_names()?)?;
        if !doc.add_host(destination.as_str())? {
            return Err(HostError::AlreadyListed(doc.host_names()?));
        }
        Ok(doc.host_names()?)
    });
    let names = match edited {
        Ok(names) => names,
        Err(HostError::AlreadyListed(names)) => {
            let message = format!("`{destination}` was already listed");
            streams.aside("already_listed", &message);
            names
        }
        Err(err) => return fail(streams, &err),
    };
    let code = emit_hosts(streams, names);
    if code == ExitCode::Success {
        probe(streams, &destination, ssh).await;
    }
    code
}

/// Dials `destination` once and says on stderr how it went.
///
/// A handshake and nothing more: the connection is dropped unused. A failure
/// is a warning, never an error, because a host that is down tonight is still
/// a host.
async fn probe(streams: &mut Streams<'_>, destination: &Destination, ssh: &SshOptions) {
    match Client::connect_ssh_with(destination.as_str(), ssh).await {
        Ok(client) => {
            drop(client);
            let message = format!("{destination}: shepherd answered");
            streams.aside("host_answered", &message);
        }
        Err(err) => {
            let message = format!("{err}\n{}", unreachable_hint(destination, &err));
            streams.aside("host_unreachable", &message);
        }
    }
}

/// What an operator can do about `err`, the failure of dialling `destination`.
fn unreachable_hint(destination: &Destination, err: &ConnectError) -> String {
    const KEPT: &str = "`shep host add` kept the entry.";
    const AGENT: &str = "A fleet listing runs without prompts, so the key must be in `ssh-agent`.";
    match err {
        ConnectError::Ssh {
            cause: SshCause::CommandNotFound,
            ..
        } => format!(
            "{KEPT} `shep` must be on PATH for a non-interactive ssh session, which reads the \
             shell's startup file differently from a login. Put it where \
             `ssh {destination} shep --version` finds it."
        ),
        ConnectError::Ssh {
            cause: SshCause::NoShepherd,
            ..
        } => format!(
            "{KEPT} The remote shepherd is not running. \
             `ssh {destination} shep muster` boots it from its roll."
        ),
        ConnectError::Ssh {
            cause: SshCause::Transport { .. },
            ..
        } => format!("{KEPT} {AGENT}"),
        ConnectError::HandshakeTimeout { after } => {
            format!(
                "{KEPT} `{destination}` did not answer in time ({after:?}). Check that it is up \
                 and reachable from here."
            )
        }
        _ => KEPT.to_owned(),
    }
}

/// `shep host remove <destination>`.
///
/// Exits [`ExitCode::NotFound`] for a destination that is not listed, rather
/// than exiting 0 on a no-op an operator would read as success.
fn remove(streams: &mut Streams<'_>, paths: &ShepPaths, text: &str) -> ExitCode {
    let destination = match parse_argument(streams, text) {
        Ok(destination) => destination,
        Err(code) => return code,
    };
    let edited = ShepToml::try_edit(&paths.daemon_config, |doc| {
        validated(doc.host_names()?)?;
        if !doc.remove_host(destination.as_str())? {
            return Err(HostError::Absent);
        }
        Ok(doc.host_names()?)
    });
    match edited {
        Ok(names) => emit_hosts(streams, names),
        Err(HostError::Absent) => {
            let message = format!("`{destination}` is not listed");
            streams.fail(ExitCode::NotFound, &message)
        }
        Err(err) => fail(streams, &err),
    }
}

/// `shep host list`: every entry, in file order. Creates nothing.
fn list(streams: &mut Streams<'_>, paths: &ShepPaths) -> ExitCode {
    let names = ShepToml::read_only(&paths.daemon_config)
        .map_err(HostError::from)
        .and_then(|doc| validated(doc.host_names()?));
    match names {
        Ok(names) => emit_hosts(streams, names),
        Err(err) => fail(streams, &err),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use clap::Parser as _;

    use super::*;
    use crate::cli::{Cli, Commands};

    fn paths_in(dir: &tempfile::TempDir) -> ShepPaths {
        let home = dir.path().display().to_string();
        ShepPaths::resolve(
            &move |key| (key == "SHEP_HOME").then(|| home.clone()),
            dir.path(),
        )
    }

    /// Runs `shep host <words>` as the real parser reads it, returning the
    /// exit code with what it wrote to stdout and stderr.
    async fn run(paths: &ShepPaths, fmt: Format, words: &[&str]) -> (ExitCode, String, String) {
        run_with(paths, fmt, words, &no_ssh()).await
    }

    /// An ssh whose program does not exist, so a test that is not about the
    /// dial never reaches a network and only warns.
    fn no_ssh() -> SshOptions {
        unattended(Duration::from_secs(5)).program("/nonexistent/ssh")
    }

    /// [`run`] with the ssh settings `add` dials under.
    async fn run_with(
        paths: &ShepPaths,
        fmt: Format,
        words: &[&str],
        ssh: &SshOptions,
    ) -> (ExitCode, String, String) {
        let argv = ["shep", "host"].into_iter().chain(words.iter().copied());
        let Commands::Host(args) = Cli::try_parse_from(argv).unwrap().command else {
            panic!("expected Commands::Host");
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = host_with(
            &mut Streams {
                out: &mut out,
                err: &mut err,
                style: crate::style::Presentation::BARE,
                fmt,
            },
            paths,
            &args,
            ssh,
        )
        .await;
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[tokio::test]
    async fn host_add_refuses_an_argument_the_destination_grammar_refuses_with_usage() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        for bad in ["-oProxyCommand=x", "amp2:2222", "a b", "@host", "user@"] {
            let (code, out, err) = run(&paths, Format::Json, &["add", "--", bad]).await;
            assert_eq!(code, ExitCode::Usage, "{bad}: {err}");
            assert!(out.is_empty(), "{bad}");
            assert!(err.contains("not a destination"), "{bad}: {err}");
            let (code, _, _) = run(&paths, Format::Json, &["remove", "--", bad]).await;
            assert_eq!(code, ExitCode::Usage, "{bad}");
        }
        assert!(
            !paths.daemon_config.exists(),
            "a refused argument must not create shep.toml"
        );
    }

    #[tokio::test]
    async fn host_add_then_list_prints_in_file_order_and_json_is_an_array_of_objects() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        assert_eq!(
            run(&paths, Format::Json, &["add", "zed"]).await.0,
            ExitCode::Success
        );
        assert_eq!(
            run(&paths, Format::Json, &["add", "deploy@amp2"]).await.0,
            ExitCode::Success
        );
        let (code, out, err) = run(&paths, Format::Json, &["add", "zed"]).await;
        assert_eq!(code, ExitCode::Success, "a second add is not an error");
        assert!(err.contains("already listed"), "{err}");
        let after_add: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(after_add["data"].as_array().unwrap().len(), 2, "{out}");

        let (code, out, _) = run(&paths, Format::Json, &["list"]).await;
        assert_eq!(code, ExitCode::Success);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            json["data"],
            serde_json::json!([{"host": "zed"}, {"host": "deploy@amp2"}])
        );

        let (_, table, _) = run(&paths, Format::Table, &["list"]).await;
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3, "{table}");
        assert!(lines[0].contains("HOST"), "{table}");
        assert!(
            lines[1].contains("zed") && lines[2].contains("deploy@amp2"),
            "{table}"
        );
    }

    #[tokio::test]
    async fn an_empty_list_is_one_note_in_a_table_and_an_empty_array_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        let (code, out, _) = run(&paths, Format::Table, &["list"]).await;
        assert_eq!(code, ExitCode::Success);
        assert!(
            out.contains("no hosts. `shep host add <destination>` adds one."),
            "{out}"
        );
        assert_eq!(out.lines().count(), 1, "{out}");

        let (_, out, _) = run(&paths, Format::Json, &["list"]).await;
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["data"], serde_json::json!([]));
        assert!(!paths.daemon_config.exists(), "a list creates nothing");
    }

    #[tokio::test]
    async fn host_remove_of_a_stranger_exits_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let original = "hosts = [\"amp2\"]\n";
        std::fs::write(&paths.daemon_config, original).unwrap();

        let (code, out, err) = run(&paths, Format::Json, &["remove", "amp3"]).await;

        assert_eq!(code, ExitCode::NotFound, "{err}");
        assert!(out.is_empty());
        assert!(err.contains("amp3"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&paths.daemon_config).unwrap(),
            original
        );

        let (code, out, _) = run(&paths, Format::Json, &["remove", "amp2"]).await;
        assert_eq!(code, ExitCode::Success);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["data"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn a_hand_edited_entry_that_is_not_a_destination_is_named_and_exits_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let original = "hosts = [\"amp2\", \"-oProxyCommand=evil\"]\n";
        std::fs::write(&paths.daemon_config, original).unwrap();

        for words in [&["list"][..], &["add", "amp3"], &["remove", "amp2"]] {
            let (code, out, err) = run(&paths, Format::Json, words).await;
            assert_eq!(code, ExitCode::InvalidConfig, "{words:?}: {err}");
            assert!(out.is_empty(), "{words:?}: nothing reaches stdout: {out}");
            assert!(err.contains("-oProxyCommand=evil"), "{words:?}: {err}");
        }
        assert_eq!(
            std::fs::read_to_string(&paths.daemon_config).unwrap(),
            original,
            "a refused edit leaves the file alone"
        );
    }

    #[tokio::test]
    async fn a_malformed_hosts_key_exits_invalid_config_whatever_the_verb() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        for original in ["hosts = \"amp2\"\n", "hosts = [\"amp2\", 5]\n"] {
            std::fs::write(&paths.daemon_config, original).unwrap();
            for words in [&["list"][..], &["add", "amp3"], &["remove", "amp2"]] {
                let (code, out, err) = run(&paths, Format::Json, words).await;
                assert_eq!(code, ExitCode::InvalidConfig, "{original} {words:?}: {err}");
                assert!(out.is_empty(), "{original} {words:?}: {out}");
                assert!(err.contains("hosts"), "{original} {words:?}: {err}");
            }
            assert_eq!(
                std::fs::read_to_string(&paths.daemon_config).unwrap(),
                original
            );
        }
    }

    /// The `ssh` that `host add` runs in these tests: an executable `/bin/sh`
    /// script in `dir` with `body` as its text.
    fn stand_in(dir: &tempfile::TempDir, body: &str) -> SshOptions {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.path().join("ssh");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        unattended(Duration::from_secs(10)).program(path)
    }

    /// Writes `reply.bin` into `dir`: the framed ack a shepherd answers a
    /// `Hello` with, for a stand-in to `cat`.
    fn write_reply(dir: &tempfile::TempDir) {
        use shep_core::protocol::{HelloAck, HelloReply, PROTOCOL_VERSION, encode_frame};
        let reply: HelloReply = Ok(HelloAck {
            daemon_version: "9.9.9".into(),
            protocol: PROTOCOL_VERSION,
            pid: 4242,
            min_supported: None,
        });
        let payload = encode_frame(&reply).unwrap();
        let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(&payload);
        std::fs::write(dir.path().join("reply.bin"), bytes).unwrap();
    }

    fn saved(paths: &ShepPaths) -> String {
        std::fs::read_to_string(&paths.daemon_config).unwrap()
    }

    #[tokio::test]
    async fn host_add_against_a_host_that_answers_says_so_and_exits_success() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        write_reply(&dir);
        let ssh = stand_in(&dir, "cat \"$(dirname \"$0\")/reply.bin\"\ncat >/dev/null");

        let (code, out, err) = run_with(&paths, Format::Table, &["add", "amp2"], &ssh).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert!(err.contains("amp2: shepherd answered"), "{err}");
        assert_eq!(err.lines().count(), 1, "{err}");
        assert!(out.contains("amp2"), "{out}");
        assert!(saved(&paths).contains("amp2"));
    }

    #[tokio::test]
    async fn host_add_against_an_ssh_that_fails_warns_keeps_the_entry_and_exits_success() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let ssh = stand_in(
            &dir,
            "echo 'amp2: Permission denied (publickey).' >&2\nexit 255",
        );

        let (code, out, err) = run_with(&paths, Format::Table, &["add", "amp2"], &ssh).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert!(saved(&paths).contains("hosts = [\"amp2\"]"));
        assert!(out.contains("amp2"), "stdout holds the saved row: {out}");
        assert!(err.starts_with("notice[host_unreachable]"), "{err}");
        assert!(err.contains("Permission denied (publickey)."), "{err}");
        assert!(err.contains("kept the entry"), "{err}");
        assert!(err.contains("`ssh-agent`"), "{err}");
    }

    #[tokio::test]
    async fn host_add_against_a_missing_remote_shep_names_the_path_fix() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let ssh = stand_in(&dir, "echo 'sh: shep: command not found' >&2\nexit 127");

        let (code, _, err) = run_with(&paths, Format::Table, &["add", "amp2"], &ssh).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert!(err.contains("must be on PATH"), "{err}");
        assert!(err.contains("`ssh amp2 shep --version`"), "{err}");
        assert!(saved(&paths).contains("amp2"));
    }

    #[tokio::test]
    async fn host_add_against_a_host_with_no_shepherd_names_muster() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let ssh = stand_in(&dir, "echo 'no shepherd is running' >&2\nexit 5");

        let (code, _, err) = run_with(&paths, Format::Table, &["add", "amp2"], &ssh).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert!(err.contains("not running"), "{err}");
        assert!(err.contains("`ssh amp2 shep muster`"), "{err}");
    }

    #[tokio::test]
    async fn host_add_gives_up_at_the_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let ssh = stand_in(&dir, "exec sleep 30").timeout(Some(Duration::from_millis(200)));

        let started = std::time::Instant::now();
        let (code, _, err) = run_with(&paths, Format::Table, &["add", "amp2"], &ssh).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert!(started.elapsed() < Duration::from_secs(10));
        assert!(err.contains("did not answer in time"), "{err}");
        assert!(
            !err.contains("ssh-agent"),
            "a timeout is not a key problem: {err}"
        );
        assert!(saved(&paths).contains("amp2"));
    }

    #[tokio::test]
    async fn host_add_passes_batch_mode_to_ssh() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let record =
            "for a in \"$@\"; do echo \"$a\"; done > \"$(dirname \"$0\")/args.txt\"\nexit 255";
        let ssh = stand_in(&dir, record);

        run_with(&paths, Format::Table, &["add", "amp2"], &ssh).await;

        let args = std::fs::read_to_string(dir.path().join("args.txt")).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "-T",
                "-o",
                "BatchMode=yes",
                "--",
                "amp2",
                "shep",
                "dial-stdio"
            ]
        );
    }

    #[tokio::test]
    async fn host_remove_and_host_list_never_run_ssh() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        std::fs::write(&paths.daemon_config, "hosts = [\"amp2\", \"amp3\"]\n").unwrap();
        let ran = dir.path().join("ran");
        let ssh = stand_in(&dir, &format!("touch '{}'\nexit 255", ran.display()));

        let (code, _, err) = run_with(&paths, Format::Table, &["list"], &ssh).await;
        assert_eq!(code, ExitCode::Success, "{err}");
        let (code, _, err) = run_with(&paths, Format::Table, &["remove", "amp2"], &ssh).await;
        assert_eq!(code, ExitCode::Success, "{err}");

        assert!(!ran.exists(), "ssh ran for a verb that only edits a list");
    }

    #[tokio::test]
    async fn under_json_the_unreachable_warning_is_a_stderr_notice_and_stdout_stays_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let ssh = stand_in(&dir, "echo 'Connection refused' >&2\nexit 255");

        let (code, out, err) = run_with(&paths, Format::Json, &["add", "amp2"], &ssh).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        let rows: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(rows["data"], serde_json::json!([{"host": "amp2"}]), "{out}");
        assert_eq!(out.lines().count(), 1, "{out}");
        assert_eq!(err.lines().count(), 1, "{err}");
        let notice: serde_json::Value = serde_json::from_str(&err).unwrap();
        assert_eq!(notice["notice"]["code"], "host_unreachable", "{err}");
        assert!(
            notice["notice"]["message"]
                .as_str()
                .unwrap()
                .contains("Connection refused"),
            "{err}"
        );
    }
}
