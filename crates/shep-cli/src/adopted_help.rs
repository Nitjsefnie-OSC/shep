//! Adopted dogs as part of shep's own help and completion surface.
//!
//! A dog's verb is `shep <name>` and only that: the registry in `shep.toml`
//! decides which names are dogs, and [`crate::adopted`] runs one only for a
//! token clap could not place as a verb. A manifest's `verb` says how that
//! name is described. `description` is the line `shep --help` lists it with,
//! and `subcommands` are what `shep <dog> --help` prints and what a
//! completion script offers.
//!
//! Every read goes to the manifest cache `shep adopt` wrote, never to the
//! binary: a help screen must not spawn a dog. A dog with nothing cached, or
//! which declared no `verb`, is listed with nothing beside its name.

use std::ffi::OsString;
use std::path::Path;

use shep_core::dogs::manifest::{Subcommand, Verb};
use shep_core::paths::ShepPaths;

use crate::cli::HELP_TEMPLATE;
use crate::commands::dogs::{cached_manifest, collides_with_a_verb};
use crate::commands::shep_toml::ShepToml;
use crate::exit::ExitCode;
use crate::{cli, home};

/// The `HELP_TEMPLATE` group line adopted dogs join, and the heading
/// `cli::help`'s `HELP_GROUPS` files `dogs`, `adopt` and `whistle` under.
const DOG_GROUP_HEADING: &str = "Dogs and agents";

/// The column every group line's verbs start at, in `HELP_TEMPLATE`: the
/// longest heading plus two spaces.
///
/// A literal rather than a computed one because the template is hand-written
/// copy and this is its layout, not data derived from it.
/// `the_dog_lines_line_up_under_the_group_verbs` pins it against the real
/// template.
const VERB_COLUMN: usize = 17;

/// One adopted dog, as `--help` and a completion script need it.
///
/// `Debug` is derived and needs no redaction: a verb's description and its
/// subcommands' one-liners are copy an operator is meant to read. The half of
/// a manifest that can carry a credential, its `defaults`, never reaches
/// here.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AdoptedVerb {
    /// The name the operator types: `shep <name>`.
    pub name: String,
    /// The manifest's `verb.description`. `None` for a dog that declared
    /// none, which is every dog adopted before manifests existed.
    pub description: Option<String>,
    /// The manifest's `verb.subcommands`, empty for a dog that declared none.
    pub subcommands: Vec<Subcommand>,
}

/// Every dog `[daemon] adopted_dogs` records, in name order, each with what
/// its cached manifest declares.
///
/// `shep.toml` is read read-only and creates nothing, so a listing costs a
/// mistyped verb or a `--help` no filesystem change. A `shep.toml` that
/// cannot be read or parsed lists nothing rather than failing the invocation
/// that only wanted to print help: the verb that reads the broken file has
/// its own refusal, and `--help` is not it.
///
/// A name a verb or alias already owns is left out. `shep adopt` refuses such
/// a name, but a hand-written `shep.toml` can hold one, and clap's verb wins
/// the token every time, so listing the dog would promise a command that can
/// never run.
#[must_use]
pub(crate) fn adopted_verbs(paths: &ShepPaths) -> Vec<AdoptedVerb> {
    let Ok(config) = ShepToml::read_only(&paths.daemon_config) else {
        return Vec::new();
    };

    let mut verbs: Vec<AdoptedVerb> = config
        .adopted_dog_names()
        .into_iter()
        .filter(|name| !collides_with_a_verb(name))
        .map(|name| {
            let verb = config
                .adopted_dog_path(&name)
                .as_deref()
                .and_then(|path| cached_manifest(path, &paths.home))
                .and_then(|manifest| manifest.verb);
            AdoptedVerb {
                description: verb.as_ref().map(|verb| verb.description.clone()),
                subcommands: verb.map_or_else(Vec::new, |verb| verb.subcommands),
                name,
            }
        })
        .collect();
    verbs.sort_by(|a, b| a.name.cmp(&b.name));
    verbs
}

/// Whether this argv can want the top-level help, which is the only render
/// the dog listing changes.
///
/// A token scan rather than a parse, because building the listing costs a
/// read of `shep.toml` and a sha256 of every adopted dog's binary, and a
/// plain `shep stop web` must not pay that. It answers `true` for a `--help`
/// aimed at one verb, and for a token that is only somebody's argument: both
/// cost the listing and change nothing, since a subcommand renders with
/// clap's own template rather than this one.
#[must_use]
pub(crate) fn top_level_help_asked(argv: &[OsString]) -> bool {
    argv.len() == 1
        || argv[1..]
            .iter()
            .any(|arg| arg == "-h" || arg == "--help" || arg == "help")
}

/// The `$SHEP_HOME` this argv names, for an invocation clap never parsed to
/// the end.
///
/// `None` when the home cannot be resolved at all, which is an ordinary
/// answer here and never a refusal: `--help` prints whatever it would have
/// printed.
pub(crate) fn paths_from_argv(argv: &[OsString]) -> Option<ShepPaths> {
    let global = cli::GlobalArgs {
        host: None,
        home: crate::adopted::home_before(&argv[1..]),
        format: cli::Format::Table,
        quiet: false,
        style: None,
    };
    home::resolve_paths(&global).ok()
}

/// Render `verbs` into the Dogs group of `command`'s help template.
///
/// Does nothing when there are no dogs, so a home that has adopted none
/// renders `HELP_TEMPLATE` byte for byte.
///
/// # Panics
///
/// Panics when `HELP_TEMPLATE` has lost the `Dogs and agents` heading the
/// group table files `dogs`, `adopt` and `whistle` under, which is the copy
/// `cli::help`'s tests pin. The panic names the heading.
#[track_caller]
pub(crate) fn list_in_help(command: &mut clap::Command, verbs: &[AdoptedVerb]) {
    if verbs.is_empty() {
        return;
    }

    let mut lines: Vec<String> = HELP_TEMPLATE.lines().map(str::to_string).collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with(DOG_GROUP_HEADING))
        .expect("HELP_TEMPLATE has a Dogs group");
    lines.splice(at + 1..at + 1, dog_lines(verbs));
    *command = std::mem::take(command).help_template(lines.join("\n"));
}

/// The lines the dogs get under the group heading: each name in the column
/// every group's verbs sit in, each description two columns past the longest
/// name. A dog with no description gets its name and nothing else.
fn dog_lines(verbs: &[AdoptedVerb]) -> Vec<String> {
    let widest = verbs.iter().map(|verb| verb.name.len()).max();
    verbs
        .iter()
        .map(|verb| {
            let mut line = " ".repeat(VERB_COLUMN);
            line.push_str(&verb.name);
            if let Some(description) = &verb.description {
                let pad = widest.unwrap_or(0) + 2 - verb.name.len();
                line.push_str(&" ".repeat(pad));
                line.push_str(description);
            }
            line
        })
        .collect()
}

/// Add each dog, and each declared subcommand under it, to a command tree
/// built only to generate a completion script.
///
/// This tree is never parsed. `shep <dog>` still reaches the dog through
/// clap's unknown-subcommand path, which is what keeps a dog's argv a
/// pass-through and keeps a manifest from claiming a verb.
pub(crate) fn with_completion_entries(
    command: clap::Command,
    verbs: &[AdoptedVerb],
) -> clap::Command {
    verbs.iter().fold(command, |command, verb| {
        command.subcommand(completion_entry(verb))
    })
}

/// The completion tree's entry for one dog.
fn completion_entry(verb: &AdoptedVerb) -> clap::Command {
    let mut command = clap::Command::new(verb.name.clone())
        .about(verb.description.clone().unwrap_or_default())
        .disable_help_subcommand(true);
    for sub in &verb.subcommands {
        command = command.subcommand(clap::Command::new(sub.name.clone()).about(sub.about.clone()));
    }
    command
}

/// `shep <dog> --help`, rendered from the manifest.
///
/// `None` for every other invocation, and for a dog that declares no
/// subcommands: `--help` reaches those dogs' own binaries, which is what a
/// dog that writes its own help gets.
pub(crate) fn describe_dog(
    name: &str,
    path: &Path,
    home: &Path,
    argv: &[OsString],
) -> Option<ExitCode> {
    if !matches!(argv, [flag] if flag == "--help" || flag == "-h") {
        return None;
    }
    let verb = cached_manifest(path, home)?.verb?;
    if verb.subcommands.is_empty() {
        return None;
    }
    // A failed write is a closed stdout, and there is nothing left to say
    // about it: `shep <dog> --help | head` is the ordinary reason.
    let _ = dog_help(name, &verb).print_long_help();
    Some(ExitCode::Success)
}

/// The help page for one dog's verb: its description, its declared
/// subcommands, and the one fact a manifest cannot state, that everything
/// else after the name goes to the dog.
fn dog_help(name: &str, verb: &Verb) -> clap::Command {
    let command = clap::Command::new(name.to_owned())
        .bin_name(format!("shep {name}"))
        .about(verb.description.clone())
        .disable_help_subcommand(true)
        .after_help(format!(
            "Anything else after `shep {name}` goes to the dog unchanged."
        ));
    verb.subcommands.iter().fold(command, |command, sub| {
        command.subcommand(clap::Command::new(sub.name.clone()).about(sub.about.clone()))
    })
}

/// [`dog_help`] rendered, for the tests that read it rather than print it.
#[cfg(test)]
pub(crate) fn dog_help_text(name: &str, verb: &Verb) -> String {
    dog_help(name, verb).render_long_help().to_string()
}

#[cfg(all(test, unix))]
mod tests;
