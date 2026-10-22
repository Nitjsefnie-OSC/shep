//! `--host`: which verbs may reach another machine's shepherd, and how a
//! verb dials the one it is told to.
//!
//! Every verb is classified by [`host_scope`], and a [`HostScope::LocalOnly`]
//! verb is refused before it touches anything. The failure this exists to
//! prevent is a verb that ignores `--host` and acts on this machine.

use shep_client::SshOptions;
use shep_core::paths::ShepPaths;

use crate::cli::{Commands, Format};
use crate::client::emit_error_locked;
use crate::exit::ExitCode;

/// What `--host` means to a verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostScope {
    /// The verb dials the host named by `--host`.
    Reaches,
    /// The verb acts on this machine, so `--host` is refused.
    LocalOnly,
}

/// Whether `command` reaches the host `--host` names.
///
/// No wildcard arm: a new verb fails to compile until someone decides, and
/// each slice that lets a verb reach a host moves it to
/// [`HostScope::Reaches`] here and nowhere else.
pub(crate) fn host_scope(command: &Commands) -> HostScope {
    match command {
        Commands::Flock(_) => HostScope::Reaches,
        Commands::Start(_)
        | Commands::Add(_)
        | Commands::Serve(_)
        | Commands::Stop(_)
        | Commands::Restart(_)
        | Commands::Reload(_)
        | Commands::Delete(_)
        | Commands::Stock(_)
        | Commands::Dogs(_)
        | Commands::Enable(_)
        | Commands::Disable(_)
        | Commands::Adopt(_)
        | Commands::Rehome(_)
        | Commands::Describe(_)
        | Commands::Trigger(_)
        | Commands::Signal(_)
        | Commands::Whisper(_)
        | Commands::Fold(_)
        | Commands::Bleats(_)
        | Commands::Lookout(_)
        | Commands::Whistle
        | Commands::Reopen(_)
        | Commands::Flush(_)
        | Commands::Barks(_)
        | Commands::Set(_)
        | Commands::Get(_)
        | Commands::Unset(_)
        | Commands::Secret(_)
        | Commands::Copy(_)
        | Commands::Ping
        | Commands::Kill
        | Commands::Save
        | Commands::Muster
        | Commands::Init(_)
        | Commands::Runtime(_)
        | Commands::Dev(_)
        | Commands::Import(_)
        | Commands::Startup(_)
        | Commands::Unstartup(_)
        | Commands::Completions(_)
        | Commands::Welcome
        | Commands::Style(_)
        | Commands::Thatlldo(_)
        | Commands::Daemon(_)
        | Commands::Dog(_)
        | Commands::Schema
        | Commands::DialStdio => HostScope::LocalOnly,
    }
}

/// The one refusal every [`HostScope::LocalOnly`] verb gets.
///
/// Names no supported verb, so widening [`host_scope`] never edits it.
pub(crate) const HOST_REFUSAL: &str = "`--host` is not supported by this command yet, \
     and it would have run on this machine, not on the host";

/// Refuses a [`HostScope::LocalOnly`] verb: one error, exit
/// [`ExitCode::Usage`], and nothing touched.
pub(crate) fn refuse_host(fmt: Format) -> ExitCode {
    emit_error_locked(fmt, ExitCode::Usage, HOST_REFUSAL);
    ExitCode::Usage
}

/// Where a verb's shepherd is: this machine's socket, or a host over ssh.
///
/// Only a [`HostScope::Reaches`] verb is ever given [`Self::Host`].
#[derive(Debug)]
pub(crate) enum Link<'a> {
    /// The shepherd at `paths.socket`.
    Local(&'a ShepPaths),
    /// The shepherd on `dest`, dialled through `options`.
    Host {
        /// The ssh destination, already checked by `--host`'s parser.
        dest: &'a str,
        /// How ssh is run.
        options: SshOptions,
    },
}

impl<'a> Link<'a> {
    /// The link `--host` selects: the host when one is named, else `paths`.
    pub(crate) fn new(paths: &'a ShepPaths, host: Option<&'a str>) -> Self {
        match host {
            Some(dest) => Self::Host {
                dest,
                options: SshOptions::default(),
            },
            None => Self::Local(paths),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::*;
    use crate::cli::Cli;

    /// One argv per verb, first word the verb's name. A new verb fails
    /// [`every_verb_has_a_sample_argv_here`] until it has a row.
    const SAMPLES: &[&[&str]] = &[
        &["start", "web"],
        &["add", "web"],
        &["serve", "dist"],
        &["stop", "web"],
        &["restart", "web"],
        &["reload", "web"],
        &["delete", "web"],
        &["stock", "web", "2"],
        &["flock"],
        &["dogs"],
        &["enable", "metrics"],
        &["disable", "metrics"],
        &["adopt", "/bin/true"],
        &["rehome", "metrics"],
        &["describe", "web"],
        &["trigger", "web", "run"],
        &["signal", "web", "TERM"],
        &["whisper", "web", "hi"],
        &["fold", "web"],
        &["bleats", "web"],
        &["lookout"],
        &["whistle"],
        &["reopen"],
        &["flush", "web"],
        &["barks"],
        &["set", "k", "v"],
        &["get", "k"],
        &["unset", "k"],
        &["secret", "list"],
        &["copy", "web", "web2"],
        &["ping"],
        &["kill"],
        &["save"],
        &["muster"],
        &["init"],
        &["runtime"],
        &["dev"],
        &["import", "env", "--app", "web", ".env"],
        &["startup"],
        &["unstartup"],
        &["completions", "bash"],
        &["welcome"],
        &["style"],
        &["thatlldo", "web"],
        &["daemon"],
        &["dog", "metrics"],
        &["schema"],
        &["dial-stdio"],
    ];

    fn parsed_with_host(sample: &[&str]) -> Cli {
        let argv: Vec<&str> = ["shep", "--host", "amp2"]
            .into_iter()
            .chain(sample.iter().copied())
            .collect();
        Cli::try_parse_from(&argv).unwrap_or_else(|err| panic!("{argv:?} must parse: {err}"))
    }

    #[test]
    fn flock_is_the_only_verb_that_reaches_a_host() {
        for sample in SAMPLES {
            let cli = parsed_with_host(sample);
            let want = if sample[0] == "flock" {
                HostScope::Reaches
            } else {
                HostScope::LocalOnly
            };
            assert_eq!(host_scope(&cli.command), want, "{sample:?}");
        }
    }

    /// A verb added to the tree without a row here would be classified by
    /// the compiler but never parsed with `--host`, so the table is held
    /// against the tree.
    #[test]
    fn every_verb_has_a_sample_argv_here() {
        let mut in_tree: Vec<String> = Cli::command()
            .get_subcommands()
            .map(|sub| sub.get_name().to_owned())
            .filter(|name| name != "help")
            .collect();
        let mut sampled: Vec<String> = SAMPLES.iter().map(|row| row[0].to_owned()).collect();
        in_tree.sort();
        sampled.sort();
        assert_eq!(sampled, in_tree);
    }
}
