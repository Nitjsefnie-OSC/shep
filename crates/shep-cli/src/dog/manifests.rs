//! What each built-in dog declares, without a probe: a built-in dog is this
//! binary, so its manifest is one call away.
//!
//! The permissions are the whole of what the dog sends the shepherd. A dog
//! that sent a request outside them would be refused once its grants are
//! enforced, so a change to what [`metrics`](super::metrics) or
//! [`bark`](super::bark) asks for changes the list here, and
//! `tests/cli_e2e/builtin_grants.rs` runs each dog with only these grants.

use shep_core::dogs::manifest::{Manifest, Permission};
use shep_core::protocol::PROTOCOL_VERSION;

use super::BuiltInDog;

/// The manifest of the built-in dog `dog`.
pub(super) fn manifest_of(dog: BuiltInDog) -> Manifest {
    let mut manifest = Manifest::new(dog.name(), env!("CARGO_PKG_VERSION"), PROTOCOL_VERSION);
    manifest.permissions = match dog {
        BuiltInDog::Metrics => metrics(),
        BuiltInDog::Bark => bark(),
    };
    manifest
}

/// `ListFlock` for each scrape and export, and the two subscriptions of
/// [`metrics::feed`](super::metrics). Its own section is always allowed.
fn metrics() -> Vec<Permission> {
    vec![
        Permission::new(
            "flock.read",
            true,
            "lists the flock for each scrape and each export",
        ),
        Permission::new(
            "events:channel.metric",
            true,
            "keeps the samples sheep push over the shepherd channel",
        ),
        Permission::new(
            "events:process.",
            true,
            "learns when a sheep starts, stops or restarts, so its samples go stale",
        ),
    ]
}

/// `ListFlock` for its reconciliation poll, and the subscriptions its rules
/// read. Its own section and its own config topic are always allowed.
fn bark() -> Vec<Permission> {
    vec![
        Permission::new(
            "flock.read",
            true,
            "polls the flock for what the event bus missed",
        ),
        Permission::new(
            "events:process.",
            true,
            "hears sheep start, stop, restart and exit, which most rules fire on",
        ),
        Permission::new(
            "events:log.",
            false,
            "reads the log lines a log rule matches; asked only when a log rule is set",
        ),
    ]
}
