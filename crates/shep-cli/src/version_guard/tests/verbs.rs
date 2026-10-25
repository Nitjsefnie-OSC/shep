//! Which guard each verb gets, held against the two lists the module spells:
//! the recovery verbs a skew must never refuse, and the read verbs it warns
//! about instead.

use super::*;

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
        VersionGuard::Warn
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

/// Fails if a verb leaves [`READ_VERBS`], or arrives without being listed
/// there.
#[test]
fn every_read_verb_is_one_of_the_documented_read_verbs() {
    for (argv, name) in [
        (&["flock"][..], "flock"),
        (&["describe", "web"], "describe"),
        (&["bleats", "web"], "bleats"),
        (&["bleats", "web", "--no-follow"], "bleats"),
    ] {
        let command = verb(argv);
        let found = read_verb(&command).unwrap_or_else(|| panic!("{command:?} must stay a read"));
        assert_eq!(found, name);
        assert!(
            READ_VERBS.contains(&found),
            "{found} warns but is undocumented"
        );
        assert_eq!(VersionGuard::for_command(&command), VersionGuard::Warn);
    }
    assert_eq!(
        read_verb(&Commands::Kill),
        None,
        "a recovery verb is not a read verb"
    );
}

/// A verb that writes must not be widened into a warning by accident: each
/// of these changes the flock, or reaches a surface that can.
#[test]
fn every_verb_that_changes_the_flock_still_enforces() {
    for argv in [
        &["start", "web"][..],
        &["add", "web"],
        &["stop", "web"],
        &["restart", "web"],
        &["reload", "web"],
        &["delete", "web"],
        &["stock", "web", "2"],
        &["enable", "web"],
        &["disable", "web"],
        &["trigger", "web", "go"],
        &["signal", "web", "TERM"],
        &["whisper", "web", "hi"],
        &["reopen", "web"],
        &["flush", "web"],
        &["save"],
        &["muster"],
        &["copy", "web", "web2"],
        &["kv", "set", "key", "value"],
        &["set", "key", "value"],
    ] {
        let command = verb(argv);
        assert_eq!(
            VersionGuard::for_command(&command),
            VersionGuard::Enforce,
            "{argv:?}"
        );
    }
}
