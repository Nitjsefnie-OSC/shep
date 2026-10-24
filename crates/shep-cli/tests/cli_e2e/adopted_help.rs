//! Adopted dogs inside shep's own help and completion surface: the Dogs
//! group of `shep --help`, a dog's own `--help`, and a completion script.

use super::*;

/// A dog that answers `--shep-manifest` with a `verb`, and marks the fact
/// that it ran at all in `marker`.
#[cfg(unix)]
fn declaring_dog(dir: &TempDir, marker: &Path) -> PathBuf {
    let body = format!(
        "#!/bin/sh\nif [ \"$1\" = --shep-manifest ]; then\n  echo '{}'\n  exit 0\nfi\necho ran > \"{}\"\nexit 0\n",
        r#"{"manifest":1,"name":"shep-otel","version":"0.2.0","protocol":12,"verb":{"description":"exports traces to an OTLP collector","subcommands":[{"name":"status","about":"show pipeline status"},{"name":"drain","about":"empty the export queue"}]}}"#,
        marker.display(),
    );
    write_script(dir, "shep-otel.sh", &body)
}

/// A dog that answers nothing, which is every dog adopted before manifests.
#[cfg(unix)]
fn silent_dog(dir: &TempDir, marker: &Path) -> PathBuf {
    let body = format!("#!/bin/sh\necho ran > \"{}\"\nexit 0\n", marker.display());
    write_script(dir, "shep-lr.sh", &body)
}

#[cfg(unix)]
fn adopt(home: &Path, binary: &Path, name: &str) -> Output {
    Command::cargo_bin("shep")
        .unwrap()
        .arg("--home")
        .arg(home)
        .arg("adopt")
        .arg(binary)
        .arg("--name")
        .arg(name)
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap()
}

#[cfg(unix)]
fn run(home: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::cargo_bin("shep").unwrap();
    cmd.arg("--home").arg(home);
    for arg in args {
        cmd.arg(arg);
    }
    cmd.timeout(CMD_TIMEOUT).output().unwrap()
}

#[cfg(unix)]
/// The acceptance contract: an adopted dog is listed in the Dogs group of
/// `shep --help`, with the description its manifest declared, under the
/// group's own verbs and not in a section of its own.
#[test]
fn shep_help_lists_an_adopted_dog_with_its_description() {
    let home = TempDir::new().unwrap();
    let marker = home.path().join("ran");
    let binary = declaring_dog(&home, &marker);

    assert_success(&adopt(home.path(), &binary, "otel"));

    let help = run(home.path(), &["--help"]);
    assert_success(&help);
    let help = String::from_utf8(help.stdout).unwrap();
    let group = help
        .lines()
        .position(|line| line.starts_with("Dogs and agents"))
        .expect("`--help` still has a Dogs group");

    assert_eq!(
        help.lines().skip(group).take(2).collect::<Vec<_>>(),
        vec![
            "Dogs and agents  dogs adopt whistle",
            "                 otel  exports traces to an OTLP collector",
        ],
        "the dog joins the group it belongs to: {help}"
    );
    assert!(!marker.exists(), "`--help` must never run the dog it lists");
}

#[cfg(unix)]
/// A dog with no manifest is still part of the listing, with nothing beside
/// its name: the listing says what is known and is silent about what isn't.
#[test]
fn shep_help_lists_an_undeclared_dog_with_no_description() {
    let home = TempDir::new().unwrap();
    let marker = home.path().join("ran");
    let binary = silent_dog(&home, &marker);

    assert_success(&adopt(home.path(), &binary, "lr"));

    let help = String::from_utf8(run(home.path(), &["--help"]).stdout).unwrap();
    assert!(
        help.lines().any(|line| line == "                 lr"),
        "an undeclared dog is listed bare:\n{help}"
    );
}

#[cfg(unix)]
/// `shep <dog> --help` is shep's page for a dog that declared its
/// subcommands, and the dog never runs to answer it.
#[test]
fn a_declared_dogs_own_help_lists_its_subcommands() {
    let home = TempDir::new().unwrap();
    let marker = home.path().join("ran");
    let binary = declaring_dog(&home, &marker);

    assert_success(&adopt(home.path(), &binary, "otel"));

    let help = run(home.path(), &["otel", "--help"]);
    assert_success(&help);
    let help = String::from_utf8(help.stdout).unwrap();

    assert!(help.contains("Usage: shep otel [COMMAND]"), "{help}");
    assert!(
        help.contains("exports traces to an OTLP collector"),
        "{help}"
    );
    assert!(help.contains("status  show pipeline status"), "{help}");
    assert!(help.contains("drain"), "{help}");
    assert!(help.contains("empty the export queue"), "{help}");
    assert!(
        help.contains("Anything else after `shep otel` goes to the dog unchanged."),
        "{help}"
    );
    assert!(
        !marker.exists(),
        "the dog was spawned to answer --help: {help}"
    );
}

#[cfg(unix)]
/// The other half of the rule: a dog that declared nothing keeps the
/// `--help` it was always given, passed through like every other argument.
#[test]
fn an_undeclared_dogs_help_flag_still_reaches_the_dog() {
    let home = TempDir::new().unwrap();
    let marker = home.path().join("ran");
    let binary = silent_dog(&home, &marker);

    assert_success(&adopt(home.path(), &binary, "lr"));

    assert_success(&run(home.path(), &["lr", "--help"]));
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        "ran",
        "the dog answers its own --help"
    );
}

#[cfg(unix)]
/// Completion offers the dog and what it declared. The script is generated
/// from the registry as it stands at generation time, which is what a static
/// script can be.
#[test]
fn completions_offer_an_adopted_dog_and_its_subcommands() {
    let home = TempDir::new().unwrap();
    let marker = home.path().join("ran");
    let binary = declaring_dog(&home, &marker);

    assert_success(&adopt(home.path(), &binary, "otel"));

    let script = String::from_utf8(run(home.path(), &["completions", "bash"]).stdout).unwrap();
    for word in ["otel", "status", "drain"] {
        assert!(script.contains(word), "`{word}` is missing from the script");
    }
    assert!(!marker.exists(), "generating a script must not run the dog");
}

#[cfg(unix)]
/// A dog is a name the registry gave, never one a manifest claimed: a
/// manifest's `subcommands` complete under `shep <dog>`, and nothing in a
/// manifest puts a word in shep's own verb list.
#[test]
fn a_manifest_cannot_put_a_word_in_the_verb_listing() {
    let home = TempDir::new().unwrap();
    let marker = home.path().join("ran");
    let binary = declaring_dog(&home, &marker);

    assert_success(&adopt(home.path(), &binary, "otel"));

    let help = String::from_utf8(run(home.path(), &["--help"]).stdout).unwrap();
    let groups = help
        .lines()
        .filter(|line| {
            line.starts_with("Run things")
                || line.starts_with("See what's up")
                || line.starts_with("The shepherd")
        })
        .collect::<Vec<_>>();
    for line in groups {
        assert!(
            !line.contains("status") && !line.contains("drain"),
            "{line}"
        );
    }

    // `shep status` is not a verb: it reaches clap's unknown-verb error.
    let unknown = run(home.path(), &["status"]);
    assert_eq!(
        unknown.status.code(),
        Some(2),
        "clap's own usage exit code: {unknown:?}"
    );
}
