//! Adding and removing a `hosts` entry: a top-level array, not a table.

use super::*;

/// fails if the writer round-trips through a plain `toml::Table`, or puts
/// `hosts` under `[daemon]`, or lets it land after a table (where TOML would
/// read it as a key of that table).
#[test]
fn add_host_writes_a_top_level_array_and_keeps_comments_and_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shep.toml");
    let original =
        "# the shepherd's own knobs\n[daemon]\nlog_level = \"info\"  # chatty\nlog_json = false\n";
    std::fs::write(&path, original).unwrap();

    assert!(ShepToml::try_edit(&path, |doc| doc.add_host("amp2")).unwrap());
    assert!(ShepToml::try_edit(&path, |doc| doc.add_host("deploy@build")).unwrap());

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("# the shepherd's own knobs"));
    assert!(written.contains("# chatty"));
    assert!(
        written.find("log_level").unwrap() < written.find("log_json").unwrap(),
        "key order survives"
    );
    let cfg = DaemonConfig::load(Some(&written), &|_| None).unwrap();
    assert_eq!(cfg.hosts, ["amp2", "deploy@build"], "top level: {written}");
    assert!(!cfg.daemon.log_json);
    assert_eq!(
        ShepToml::read_only(&path).unwrap().host_names(),
        ["amp2", "deploy@build"]
    );
}

#[test]
fn add_host_twice_keeps_one_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shep.toml");

    assert!(ShepToml::try_edit(&path, |doc| doc.add_host("amp2")).unwrap());
    assert!(!ShepToml::try_edit(&path, |doc| doc.add_host("amp2")).unwrap());

    assert_eq!(ShepToml::read_only(&path).unwrap().host_names(), ["amp2"]);
}

#[test]
fn remove_host_leaves_every_other_key_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shep.toml");
    let original = "hosts = [\"amp2\", \"amp3\"]  # the fleet\n[daemon]\nlog_json = true\n";
    std::fs::write(&path, original).unwrap();

    assert!(ShepToml::edit(&path, |doc| doc.remove_host("amp2")).unwrap());
    assert!(!ShepToml::edit(&path, |doc| doc.remove_host("stranger")).unwrap());

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.contains("[daemon]\nlog_json = true"), "{written}");
    assert!(written.contains("# the fleet"), "{written}");
    assert_eq!(ShepToml::read_only(&path).unwrap().host_names(), ["amp3"]);
}

#[test]
fn a_hosts_key_that_is_not_an_array_is_wrong_shape_and_the_file_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shep.toml");
    std::fs::write(&path, "hosts = \"amp2\"\n").unwrap();

    let err = ShepToml::try_edit(&path, |doc| doc.add_host("amp3")).unwrap_err();

    assert!(
        matches!(err, ShepTomlError::WrongShape { key: "hosts", .. }),
        "{err}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hosts = \"amp2\"\n");
}
