//! `shep host add` / `remove` / `list`: the CLI half of `shep.toml`'s
//! top-level `hosts` array.
//!
//! No [`Client`](shep_client::Client) anywhere in this module, and no ssh
//! either: the verb edits a list, and a later slice reads it. Every entry is
//! checked against [`Destination`] before this module hands it on, so a
//! hand-edited `-oProxyCommand=...` is named and refused rather than skipped
//! or passed along.

use shep_core::destination::{Destination, DestinationError};
use shep_core::paths::ShepPaths;

use crate::cli::{Format, HostArgs, HostCommand};
use crate::commands::shep_toml::{ShepToml, ShepTomlError};
use crate::exit::ExitCode;
use crate::output::{HostEntry, HostRows, Streams, emit, write_outcome};

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
pub fn host(streams: &mut Streams<'_>, paths: &ShepPaths, args: &HostArgs) -> ExitCode {
    match &args.command {
        HostCommand::Add { destination } => add(streams, paths, destination),
        HostCommand::Remove { destination } => remove(streams, paths, destination),
        HostCommand::List => list(streams, paths),
    }
}

/// `shep host add <destination>`.
///
/// Idempotent: a destination already listed exits [`ExitCode::Success`],
/// prints the list, and says so on stderr.
fn add(streams: &mut Streams<'_>, paths: &ShepPaths, text: &str) -> ExitCode {
    let destination = match parse_argument(streams, text) {
        Ok(destination) => destination,
        Err(code) => return code,
    };
    let edited = ShepToml::try_edit(&paths.daemon_config, |doc| {
        validated(doc.host_names())?;
        if !doc.add_host(destination.as_str())? {
            return Err(HostError::AlreadyListed(doc.host_names()));
        }
        Ok(doc.host_names())
    });
    match edited {
        Ok(names) => emit_hosts(streams, names),
        Err(HostError::AlreadyListed(names)) => {
            let message = format!("`{destination}` was already listed");
            streams.aside("already_listed", &message);
            emit_hosts(streams, names)
        }
        Err(err) => fail(streams, &err),
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
        validated(doc.host_names())?;
        if !doc.remove_host(destination.as_str()) {
            return Err(HostError::Absent);
        }
        Ok(doc.host_names())
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
        .and_then(|doc| validated(doc.host_names()));
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
    fn run(paths: &ShepPaths, fmt: Format, words: &[&str]) -> (ExitCode, String, String) {
        let argv = ["shep", "host"].into_iter().chain(words.iter().copied());
        let Commands::Host(args) = Cli::try_parse_from(argv).unwrap().command else {
            panic!("expected Commands::Host");
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = host(
            &mut Streams {
                out: &mut out,
                err: &mut err,
                style: crate::style::Presentation::BARE,
                fmt,
            },
            paths,
            &args,
        );
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn host_add_refuses_an_argument_the_destination_grammar_refuses_with_usage() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        for bad in ["-oProxyCommand=x", "amp2:2222", "a b", "@host", "user@"] {
            let (code, out, err) = run(&paths, Format::Json, &["add", "--", bad]);
            assert_eq!(code, ExitCode::Usage, "{bad}: {err}");
            assert!(out.is_empty(), "{bad}");
            assert!(err.contains("not a destination"), "{bad}: {err}");
            let (code, _, _) = run(&paths, Format::Json, &["remove", "--", bad]);
            assert_eq!(code, ExitCode::Usage, "{bad}");
        }
        assert!(
            !paths.daemon_config.exists(),
            "a refused argument must not create shep.toml"
        );
    }

    #[test]
    fn host_add_then_list_prints_in_file_order_and_json_is_an_array_of_objects() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        assert_eq!(
            run(&paths, Format::Json, &["add", "zed"]).0,
            ExitCode::Success
        );
        assert_eq!(
            run(&paths, Format::Json, &["add", "deploy@amp2"]).0,
            ExitCode::Success
        );
        let (code, out, err) = run(&paths, Format::Json, &["add", "zed"]);
        assert_eq!(code, ExitCode::Success, "a second add is not an error");
        assert!(err.contains("already listed"), "{err}");
        let after_add: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(after_add["data"].as_array().unwrap().len(), 2, "{out}");

        let (code, out, _) = run(&paths, Format::Json, &["list"]);
        assert_eq!(code, ExitCode::Success);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            json["data"],
            serde_json::json!([{"host": "zed"}, {"host": "deploy@amp2"}])
        );

        let (_, table, _) = run(&paths, Format::Table, &["list"]);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 3, "{table}");
        assert!(lines[0].contains("HOST"), "{table}");
        assert!(
            lines[1].contains("zed") && lines[2].contains("deploy@amp2"),
            "{table}"
        );
    }

    #[test]
    fn an_empty_list_is_one_note_in_a_table_and_an_empty_array_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);

        let (code, out, _) = run(&paths, Format::Table, &["list"]);
        assert_eq!(code, ExitCode::Success);
        assert!(
            out.contains("no hosts. `shep host add <destination>` adds one."),
            "{out}"
        );
        assert_eq!(out.lines().count(), 1, "{out}");

        let (_, out, _) = run(&paths, Format::Json, &["list"]);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["data"], serde_json::json!([]));
        assert!(!paths.daemon_config.exists(), "a list creates nothing");
    }

    #[test]
    fn host_remove_of_a_stranger_exits_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let original = "hosts = [\"amp2\"]\n";
        std::fs::write(&paths.daemon_config, original).unwrap();

        let (code, out, err) = run(&paths, Format::Json, &["remove", "amp3"]);

        assert_eq!(code, ExitCode::NotFound, "{err}");
        assert!(out.is_empty());
        assert!(err.contains("amp3"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&paths.daemon_config).unwrap(),
            original
        );

        let (code, out, _) = run(&paths, Format::Json, &["remove", "amp2"]);
        assert_eq!(code, ExitCode::Success);
        let json: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(json["data"], serde_json::json!([]));
    }

    #[test]
    fn a_hand_edited_entry_that_is_not_a_destination_is_named_and_exits_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        let original = "hosts = [\"amp2\", \"-oProxyCommand=evil\"]\n";
        std::fs::write(&paths.daemon_config, original).unwrap();

        for words in [&["list"][..], &["add", "amp3"], &["remove", "amp2"]] {
            let (code, out, err) = run(&paths, Format::Json, words);
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

    #[test]
    fn a_hosts_key_that_is_not_an_array_exits_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(&dir);
        std::fs::write(&paths.daemon_config, "hosts = \"amp2\"\n").unwrap();

        let (code, _, err) = run(&paths, Format::Json, &["add", "amp3"]);

        assert_eq!(code, ExitCode::InvalidConfig, "{err}");
        assert!(err.contains("hosts"), "{err}");
    }
}
