//! What a dog's `[daemon.dog_grants.<name>]` entry says beyond `granted`: the
//! permissions waiting for approval and whether the operator has looked.

use super::DaemonConfig;
use super::testing::no_env;
use crate::permissions::{Grants, Permission};

fn entry(src: &str) -> super::DogGrantsSection {
    DaemonConfig::load(Some(src), &no_env)
        .unwrap()
        .daemon
        .dog_grants
        .remove("otel")
        .expect("otel has an entry")
}

#[test]
fn an_entry_with_only_granted_is_reviewed_with_nothing_waiting() {
    let otel = entry("[daemon.dog_grants.otel]\ngranted = [\"flock.read\"]\n");

    assert!(otel.reviewed);
    assert_eq!(otel.waiting, Grants::default());
    assert_eq!(otel.revoked, Grants::default());
}

#[test]
fn waiting_revoked_and_reviewed_are_read_from_the_entry() {
    let otel = entry(
        r#"
[daemon.dog_grants.otel]
granted = ["flock.read"]
waiting = ["events:process."]
revoked = ["logs.read"]
reviewed = false
"#,
    );

    assert!(!otel.reviewed);
    assert_eq!(
        otel.waiting,
        Grants::new([Permission::Events("process.".to_string())])
    );
    assert_eq!(otel.revoked, Grants::new([Permission::LogsRead]));
}

#[test]
fn a_misspelt_waiting_permission_is_a_startup_error_naming_it() {
    let src = "[daemon.dog_grants.otel]\ngranted = []\nwaiting = [\"flock.reed\"]\n";

    let err = DaemonConfig::load(Some(src), &no_env)
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("`flock.reed` is not a permission shep knows"),
        "{err}"
    );
}
