//! `shep unset` as the command line reads it.

use clap::{CommandFactory as _, Parser as _};

use crate::cli::{Cli, Commands};

/// fails if `unset` stops taking one address and the two flags `set` takes, or
/// grows a value. Dropping an override has nothing to put in its place.
#[test]
fn unset_takes_an_address_and_set_s_two_flags() {
    let parse = |argv: &[&str]| match Cli::try_parse_from(argv).unwrap().command {
        Commands::Unset(args) => args,
        other => panic!("expected unset, got {other:?}"),
    };

    let plain = parse(&["shep", "unset", "api.max_memory"]);
    assert_eq!(plain.address, "api.max_memory");
    assert!(!plain.dry_run);
    assert!(!plain.yes);

    let flagged = parse(&[
        "shep",
        "unset",
        "kelpie.worker.timeout",
        "--dry-run",
        "--yes",
    ]);
    assert!(flagged.dry_run);
    assert!(flagged.yes);

    assert!(Cli::try_parse_from(["shep", "unset"]).is_err());
    assert!(
        Cli::try_parse_from(["shep", "unset", "api.max_memory", "1G"]).is_err(),
        "an unset names what to drop and carries no value"
    );
}

/// fails if the store's old flags are accepted by the settings verb, which
/// would read `shep unset --all` as a request to clear something.
#[test]
fn the_store_s_all_flag_is_not_unset_s() {
    assert!(Cli::try_parse_from(["shep", "unset", "--all"]).is_err());
    assert!(Cli::try_parse_from(["shep", "kv", "unset", "--all"]).is_ok());
}

/// fails if `unset` goes back to being hidden from `--help`.
#[test]
fn unset_is_a_visible_verb() {
    let cmd = Cli::command();
    assert!(!cmd.find_subcommand("unset").unwrap().is_hide_set());
}
