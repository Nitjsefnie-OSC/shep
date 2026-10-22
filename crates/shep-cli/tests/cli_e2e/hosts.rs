//! `shep host`, through the real binary, with no shepherd anywhere.

use super::*;

#[cfg(unix)]
use crate::ssh_stand_in::write_ssh_stand_in;

/// The list is `shep.toml` and nothing else: add, list and remove work on a
/// home no shepherd ever touched, and none of them starts one.
///
/// `add` dials its entry once, so `ssh` is the stand-in first on `PATH`,
/// run against a remote home with no shepherd: the dial warns, and the exit
/// stays `Success`. Unix only, as the stand-in is a script.
#[cfg(unix)]
#[test]
fn host_add_list_remove_round_trip_on_a_fresh_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let remote = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    write_ssh_stand_in(bin.path(), remote.path());
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_owned())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();

    for destination in ["amp2", "deploy@build.example.com"] {
        let added = shep(home)
            .env("PATH", &path)
            .args(["host", "add", destination])
            .output()
            .unwrap();
        assert_success(&added);
        let warning = String::from_utf8(added.stderr).unwrap();
        assert!(warning.contains("host_unreachable"), "{warning}");
        assert!(warning.contains("shep muster"), "{warning}");
    }
    assert!(
        !home.join("run").join("shep.sock").exists(),
        "shep host must never start a shepherd"
    );
    let written = std::fs::read_to_string(home.join("shep.toml")).unwrap();
    assert!(
        written.contains("hosts = [\"amp2\", \"deploy@build.example.com\"]"),
        "{written}"
    );

    let listed = shep(home)
        .args(["--format", "json", "host", "list"])
        .output()
        .unwrap();
    assert_success(&listed);
    let envelope: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(
        envelope["data"],
        serde_json::json!([{"host": "amp2"}, {"host": "deploy@build.example.com"}]),
        "{envelope}"
    );

    let removed = shep(home)
        .args(["host", "remove", "amp2"])
        .output()
        .unwrap();
    assert_success(&removed);
    let missing = shep(home)
        .args(["host", "remove", "amp2"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(3), "{missing:?}");

    let refused = shep(home)
        .args(["host", "add", "--", "-oProxyCommand=x"])
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");

    let table = shep(home).args(["host", "list"]).output().unwrap();
    let table = String::from_utf8(table.stdout).unwrap();
    assert!(table.contains("deploy@build.example.com"), "{table}");
    assert!(!table.contains("amp2"), "{table}");
}
