//! Reading the grants an operator wrote under `[daemon.dog_grants.<name>]`.

use super::*;
use shep_core::permissions::{Grants, Permission};

fn doc_of(src: &str) -> (tempfile::TempDir, ShepToml) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shep.toml");
    std::fs::write(&path, src).unwrap();
    let doc = ShepToml::read_only(&path).unwrap();
    (dir, doc)
}

#[test]
fn a_dog_with_a_grants_entry_reads_what_was_written() {
    let (_dir, doc) =
        doc_of("[daemon.dog_grants.otel]\ngranted = [\"flock.read\", \"events:process.\"]\n");

    assert_eq!(
        doc.granted("otel"),
        Some(Grants::new([
            Permission::FlockRead,
            Permission::Events("process.".to_string()),
        ]))
    );
}

#[test]
fn a_dog_with_no_entry_is_undeclared_rather_than_granted_nothing() {
    let (_dir, doc) = doc_of("[daemon.dog_grants.otel]\ngranted = []\n");

    assert_eq!(doc.granted("otel"), Some(Grants::default()));
    assert_eq!(doc.granted("bark"), None);
}

#[test]
fn a_file_with_no_daemon_table_grants_nothing_to_anyone() {
    let (_dir, doc) = doc_of("");

    assert_eq!(doc.granted("otel"), None);
}
