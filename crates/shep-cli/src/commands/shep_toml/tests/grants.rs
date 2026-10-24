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
        doc.granted("otel").unwrap(),
        Some(Grants::new([
            Permission::FlockRead,
            Permission::Events("process.".to_string()),
        ]))
    );
}

#[test]
fn a_dog_with_no_entry_is_undeclared_rather_than_granted_nothing() {
    let (_dir, doc) = doc_of("[daemon.dog_grants.otel]\ngranted = []\n");

    assert_eq!(doc.granted("otel").unwrap(), Some(Grants::default()));
    assert_eq!(doc.granted("bark").unwrap(), None);
}

#[test]
fn a_file_with_no_daemon_table_grants_nothing_to_anyone() {
    let (_dir, doc) = doc_of("");

    assert_eq!(doc.granted("otel").unwrap(), None);
}

#[test]
fn a_misspelt_permission_is_refused_naming_the_dog_and_the_name() {
    let (_dir, doc) =
        doc_of("[daemon.dog_grants.otel]\ngranted = [\"flock.read\", \"flock.reed\"]\n");

    let err = doc.granted("otel").unwrap_err();

    assert!(matches!(err, ShepTomlError::BadGrants { .. }), "{err:?}");
    let shown = err.to_string();
    assert!(
        shown.contains("[daemon.dog_grants.otel] granted"),
        "{shown}"
    );
    assert!(
        shown.contains("`flock.reed` is not a permission shep knows"),
        "{shown}"
    );
}

#[test]
fn an_entry_that_is_not_a_list_of_names_is_refused_rather_than_read_as_undeclared() {
    for src in [
        "[daemon.dog_grants.otel]\ngranted = \"flock.read\"\n",
        "[daemon.dog_grants.otel]\ngranted = [1]\n",
        "[daemon.dog_grants.otel]\ngrant = [\"flock.read\"]\n",
        "[daemon.dog_grants]\notel = \"flock.read\"\n",
    ] {
        let (_dir, doc) = doc_of(src);

        assert!(
            matches!(doc.granted("otel"), Err(ShepTomlError::BadGrants { .. })),
            "{src}"
        );
    }
}

#[test]
fn another_dogs_bad_entry_does_not_refuse_this_dog() {
    let (_dir, doc) = doc_of("[daemon.dog_grants.bark]\ngranted = [\"flock.reed\"]\n");

    assert_eq!(doc.granted("otel").unwrap(), None);
}

#[test]
fn inline_tables_read_the_same_as_headed_ones() {
    for src in [
        "[daemon]\ndog_grants = { otel = { granted = [] } }\n",
        "[daemon.dog_grants]\notel = { granted = [] }\n",
        "daemon = { dog_grants = { otel = { granted = [] } } }\n",
        "[daemon.dog_grants.otel]\ngranted = []\n",
    ] {
        let (_dir, doc) = doc_of(src);

        assert_eq!(
            doc.granted("otel").unwrap(),
            Some(Grants::default()),
            "{src}"
        );
    }
}

#[test]
fn a_misspelt_name_in_an_inline_table_is_refused_too() {
    let (_dir, doc) = doc_of("[daemon]\ndog_grants = { otel = { granted = [\"flock.reed\"] } }\n");

    assert!(matches!(
        doc.granted("otel"),
        Err(ShepTomlError::BadGrants { .. })
    ));
}

#[test]
fn a_key_other_than_granted_is_refused_as_the_boot_refuses_it() {
    let (_dir, doc) =
        doc_of("[daemon.dog_grants.otel]\ngranted = [\"flock.read\"]\nnote = \"x\"\n");

    let err = doc.granted("otel").unwrap_err();

    assert!(matches!(err, ShepTomlError::BadGrants { .. }), "{err:?}");
    assert!(err.to_string().contains("`note`"), "{err}");
}
