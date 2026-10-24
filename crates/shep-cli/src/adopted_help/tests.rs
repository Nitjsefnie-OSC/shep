use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::CommandFactory as _;
use shep_core::dogs::manifest::{Manifest, Subcommand, Verb};
use shep_core::paths::ShepPaths;

use super::*;
use crate::cli::{Cli, Format, GlobalArgs};
use crate::commands::dogs::read_manifest;
use crate::commands::shep_toml::ShepToml;
use crate::home::resolve_paths;

/// Generous: the question is the answer, not the clock.
const BUDGET: Duration = Duration::from_secs(10);

/// A dog that answers `--shep-manifest` with a `verb`, and nothing else.
const DECLARING_BODY: &str = r#"if [ "$1" = --shep-manifest ]; then
  echo '{"manifest":1,"name":"shep-otel","version":"0.2.0","protocol":12,"verb":{"description":"exports traces to an OTLP collector","subcommands":[{"name":"status","about":"show pipeline status"},{"name":"flush","about":"flush pending spans"}]}}'
fi"#;

/// A dog that answers no flag at all: adopted, and undeclared.
const SILENT_BODY: &str = "";

fn write_dog(dir: &Path, name: &str, body: &str) -> PathBuf {
    let binary = dir.join(name);
    std::fs::write(&binary, format!("#!/bin/sh\n{body}\nexit 0\n")).unwrap();
    let mut mode = std::fs::metadata(&binary).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&binary, mode).unwrap();
    binary
}

/// A `$SHEP_HOME` under `dir`, resolved to the paths `adopted_verbs` reads,
/// with each `(name, body)` dog adopted into its `shep.toml` and probed the
/// way `shep adopt` probes. The manifest behind each one is therefore a
/// cached answer, not a fixture written by hand.
fn a_home_with<'a>(dir: &Path, dogs: impl IntoIterator<Item = (&'a str, &'a str)>) -> ShepPaths {
    let paths = resolve_paths(&GlobalArgs {
        host: None,
        home: Some(dir.join(".shep")),
        format: Format::Table,
        quiet: false,
        style: None,
    })
    .expect("an absolute --home resolves");

    for (name, body) in dogs {
        let binary = write_dog(dir, name, body);
        ShepToml::edit(&paths.daemon_config, |cfg| {
            cfg.adopt_dog(name, &binary, false).unwrap();
        })
        .unwrap();
        read_manifest(&binary, &paths.home, name, BUDGET).unwrap();
    }
    paths
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn listed(verbs: &[AdoptedVerb]) -> Vec<(&str, Option<&str>)> {
    verbs
        .iter()
        .map(|verb| (verb.name.as_str(), verb.description.as_deref()))
        .collect()
}

#[test]
fn a_manifest_dog_is_listed_with_its_description_and_an_undeclared_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(
        dir.path(),
        [("otel", DECLARING_BODY), ("log-rotate", SILENT_BODY)],
    );

    let verbs = adopted_verbs(&paths);

    assert_eq!(
        listed(&verbs),
        vec![
            ("log-rotate", None),
            ("otel", Some("exports traces to an OTLP collector")),
        ],
        "both dogs are listed, in name order, and only the one with a manifest carries a description"
    );
    assert_eq!(
        verbs
            .iter()
            .find(|verb| verb.name == "otel")
            .map(|verb| verb
                .subcommands
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()),
        Some(vec!["status", "flush"]),
        "the subcommands a manifest declares come with the dog"
    );
}

/// `shep adopt` refuses a name a verb owns; a hand-written `shep.toml` can
/// still hold one, and clap's verb wins the token every time.
#[test]
fn a_dog_named_like_a_verb_is_left_out_of_the_listing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(dir.path(), [("list", SILENT_BODY)]);

    assert_eq!(adopted_verbs(&paths), Vec::<AdoptedVerb>::new());
}

#[test]
fn the_dogs_join_the_dogs_group_of_the_rendered_help() {
    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(
        dir.path(),
        [("otel", DECLARING_BODY), ("log-rotate", SILENT_BODY)],
    );
    let verbs = adopted_verbs(&paths);

    let mut command = Cli::command();
    list_in_help(&mut command, &verbs);
    let help = command.render_long_help().to_string();

    let group = help
        .lines()
        .position(|line| line.starts_with(DOG_GROUP_HEADING))
        .expect("the Dogs group is still there");
    assert_eq!(
        help.lines().skip(group).take(3).collect::<Vec<_>>(),
        vec![
            "Dogs and agents  dogs adopt whistle",
            "                 log-rotate",
            "                 otel        exports traces to an OTLP collector",
        ],
        "the dogs sit under the group's own verbs, each with what it declared"
    );
}

#[test]
fn a_home_with_no_adopted_dogs_renders_the_help_template_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(dir.path(), []);
    assert!(adopted_verbs(&paths).is_empty());

    let mut command = Cli::command();
    list_in_help(&mut command, &[]);

    assert_eq!(
        command.render_long_help().to_string(),
        Cli::command().render_long_help().to_string(),
    );
}

/// The layout `dog_lines` assumes is `HELP_TEMPLATE`'s own: a group's verbs
/// start at [`VERB_COLUMN`], and a dog line that does not start there is a
/// listing that reads as a stray line.
#[test]
fn the_dog_lines_line_up_under_the_group_verbs() {
    let group = HELP_TEMPLATE
        .lines()
        .find(|line| line.starts_with(DOG_GROUP_HEADING))
        .expect("HELP_TEMPLATE has a Dogs group");
    assert_eq!(&group[VERB_COLUMN..VERB_COLUMN + 4], "dogs", "{group}");

    let lines = dog_lines(&[
        AdoptedVerb {
            name: "log-rotate".to_string(),
            description: None,
            subcommands: Vec::new(),
        },
        AdoptedVerb {
            name: "otel".to_string(),
            description: Some("exports traces".to_string()),
            subcommands: Vec::new(),
        },
    ]);
    for line in &lines {
        assert_eq!(
            &line[..VERB_COLUMN],
            " ".repeat(VERB_COLUMN).as_str(),
            "{line}"
        );
    }
    assert_eq!(lines[0].trim_start(), "log-rotate");
    assert_eq!(lines[1].trim_start(), "otel        exports traces");
}

#[test]
fn completion_scripts_offer_an_adopted_dog_and_its_declared_subcommands() {
    use clap_complete::aot::Shell;

    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(dir.path(), [("otel", DECLARING_BODY)]);
    let verbs = adopted_verbs(&paths);

    let mut command = with_completion_entries(Cli::command(), &verbs);
    let mut buf = Vec::new();
    clap_complete::aot::generate(Shell::Bash, &mut command, "shep", &mut buf);
    let script = String::from_utf8(buf).unwrap();

    for word in ["otel", "status", "flush"] {
        assert!(script.contains(word), "`{word}` is missing from the script");
    }
}

#[test]
fn describe_dog_answers_only_a_bare_help_flag_of_a_declared_dog() {
    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(dir.path(), [("otel", DECLARING_BODY), ("lr", SILENT_BODY)]);
    let declared = dir.path().join("otel");
    let silent = dir.path().join("lr");

    for argv in [
        os(&["status"]),
        os(&[]),
        os(&["--help", "status"]),
        os(&["-hx"]),
    ] {
        assert_eq!(
            describe_dog("otel", &declared, &paths.home, &argv),
            None,
            "{argv:?} is the dog's own to answer"
        );
    }

    assert_eq!(
        describe_dog("lr", &silent, &paths.home, &os(&["--help"])),
        None,
        "an undeclared dog keeps its own --help"
    );
}

#[test]
fn the_declared_subcommands_are_what_a_dogs_own_help_page_lists() {
    let mut verb = Verb::new("exports traces to an OTLP collector");
    verb.subcommands
        .push(Subcommand::new("status", "show pipeline status"));
    let help = dog_help_text("otel", &verb);

    assert!(help.contains("Usage: shep otel"), "{help}");
    assert!(
        help.contains("exports traces to an OTLP collector"),
        "{help}"
    );
    assert!(help.contains("status  show pipeline status"), "{help}");
    assert!(
        help.contains("Anything else after `shep otel` goes to the dog unchanged."),
        "{help}"
    );
    assert!(
        !help.contains("\n  help\n"),
        "a dog's verb has no shep subcommands of its own to list: {help}"
    );
}

#[test]
fn top_level_help_asked_reads_the_argv_a_parse_never_finished() {
    for argv in [
        os(&["shep"]),
        os(&["shep", "--help"]),
        os(&["shep", "-h"]),
        os(&["shep", "--home", "/tmp/s", "help"]),
        os(&["shep", "flock", "--help"]),
    ] {
        assert!(top_level_help_asked(&argv), "{argv:?}");
    }

    for argv in [
        os(&["shep", "flock"]),
        os(&["shep", "stop", "web"]),
        // A flag whose text merely contains the help flag's.
        os(&["shep", "--helper"]),
    ] {
        assert!(!top_level_help_asked(&argv), "{argv:?}");
    }
}

#[test]
fn a_manifest_that_declares_no_verb_leaves_the_dog_bare() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"if [ "$1" = --shep-manifest ]; then
  echo '{"manifest":1,"name":"shep-otel","version":"0.2.0","protocol":12}'
fi"#;
    let paths = a_home_with(dir.path(), [("otel", body)]);

    assert_eq!(listed(&adopted_verbs(&paths)), vec![("otel", None)]);
}

#[test]
fn a_recorded_binary_that_is_gone_still_lists_the_dog() {
    let dir = tempfile::tempdir().unwrap();
    let paths = a_home_with(dir.path(), [("otel", DECLARING_BODY)]);
    std::fs::remove_file(dir.path().join("otel")).unwrap();

    assert_eq!(listed(&adopted_verbs(&paths)), vec![("otel", None)]);
}

/// The cache is keyed by the binary's sha256, so a listing reads the answer
/// `shep adopt` already paid for. Nothing here spawns a dog: a help screen or
/// a completion script for a binary that runs on being run at all would run
/// it, and an adopted dog is not vetted for what it does on a `--help`.
#[test]
fn the_listing_never_runs_a_dog() {
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("runs");
    let body = format!("echo run >> \"{}\"\n{DECLARING_BODY}", counter.display());
    let paths = a_home_with(dir.path(), [("otel", body.as_str())]);
    let runs = || std::fs::read_to_string(&counter).unwrap().lines().count();
    assert_eq!(runs(), 1, "the adopt probe is the only spawn so far");

    let verbs = adopted_verbs(&paths);

    assert_eq!(
        verbs[0].description.as_deref(),
        Some("exports traces to an OTLP collector"),
        "the cached answer is what the listing reads"
    );
    assert_eq!(runs(), 1, "the listing spawned nothing");
}

/// The listing is a read of `shep.toml` and the cache. A `shep.toml` that
/// will not parse is the verb that reads it to report, not `--help`'s.
#[test]
fn an_unreadable_shep_toml_lists_nothing_rather_than_failing_the_listing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join(".shep");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("shep.toml"), "not = = toml").unwrap();
    let paths = resolve_paths(&GlobalArgs {
        host: None,
        home: Some(home),
        format: Format::Table,
        quiet: false,
        style: None,
    })
    .unwrap();

    assert_eq!(adopted_verbs(&paths), Vec::<AdoptedVerb>::new());
}

/// `Manifest` is what the cache holds and `AdoptedVerb` is what a listing
/// needs: a `defaults` section carrying a credential never travels with it.
#[test]
fn an_adopted_verb_debug_carries_no_manifest_body() {
    let mut manifest = Manifest::new("shep-otel", "0.2.0", 12);
    manifest.verb = Some(Verb::new("exports traces"));
    manifest.defaults = Some("token = \"hunter2\"\n".to_string());

    let verb = AdoptedVerb {
        name: "otel".to_string(),
        description: manifest.verb.map(|verb| verb.description),
        subcommands: Vec::new(),
    };

    assert_eq!(
        format!("{verb:?}"),
        "AdoptedVerb { name: \"otel\", description: Some(\"exports traces\"), subcommands: [] }"
    );
}
