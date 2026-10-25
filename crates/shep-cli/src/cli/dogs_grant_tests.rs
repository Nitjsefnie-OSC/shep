//! How `shep dogs` parses its two subcommands beside the listing it was.

use clap::Parser;

use super::{Cli, Commands, DogsCommand};

fn dogs(argv: &[&str]) -> super::DogsArgs {
    let full = ["shep", "dogs"].into_iter().chain(argv.iter().copied());
    let Commands::Dogs(args) = Cli::try_parse_from(full).unwrap().command else {
        panic!("expected dogs for {argv:?}");
    };
    args
}

#[test]
fn grant_and_revoke_take_a_dog_and_a_permission() {
    let granted = dogs(&["grant", "otel", "flock.read"]);
    let revoked = dogs(&["revoke", "otel", "flock.read"]);

    let (Some(DogsCommand::Grant(grant)), Some(DogsCommand::Revoke(revoke))) =
        (granted.command, revoked.command)
    else {
        panic!("expected grant and revoke");
    };
    assert_eq!(
        (grant.name.as_str(), grant.permission.as_str()),
        ("otel", "flock.read")
    );
    assert_eq!(
        (revoke.name.as_str(), revoke.permission.as_str()),
        ("otel", "flock.read")
    );
}

#[test]
fn the_listing_and_its_filter_still_parse() {
    let listing = dogs(&["--available", "spot"]);

    assert!(listing.command.is_none());
    assert!(listing.available);
    assert_eq!(listing.filter.as_deref(), Some("spot"));
    assert!(dogs(&[]).command.is_none());
}

#[test]
fn grant_without_a_permission_is_a_usage_error() {
    assert!(Cli::try_parse_from(["shep", "dogs", "grant", "otel"]).is_err());
    assert!(Cli::try_parse_from(["shep", "dogs", "revoke"]).is_err());
}

#[test]
fn a_dog_named_grant_is_reached_after_a_double_dash() {
    let listing = dogs(&["--", "grant"]);

    assert!(listing.command.is_none());
    assert_eq!(listing.filter.as_deref(), Some("grant"));
}
