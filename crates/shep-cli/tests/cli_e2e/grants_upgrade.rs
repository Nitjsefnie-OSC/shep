//! Upgrading shep or a dog never widens access silently, against a real
//! shepherd: the boot records what an enabled dog declares as granted and
//! not reviewed, a dog with no manifest is marked undeclared, and a rebuilt
//! dog's new permissions wait, with `shep dogs` marking each.

use super::*;

/// Writes a `/bin/sh` dog that declares `permissions` as its manifest, or
/// declares nothing when `None`, and otherwise sleeps.
#[cfg(unix)]
fn write_dog(dir: &TempDir, permissions: Option<&[&str]>) -> PathBuf {
    let manifest = permissions.map_or_else(String::new, |names| {
        let names: Vec<String> = names
            .iter()
            .map(|name| {
                format!(r#"{{"name":"{name}","required":true,"reason":"the test dog needs it"}}"#)
            })
            .collect();
        format!(
            r#"  --shep-manifest) echo '{{"manifest":1,"name":"grantdog","version":"1","protocol":12,"permissions":[{}]}}'; exit 0 ;;"#,
            names.join(",")
        )
    });
    let body =
        format!("#!/bin/sh\ncase \"$1\" in\n{manifest}\n  --*) exit 0 ;;\nesac\nexec sleep 60\n");
    write_script(dir, "grantdog.sh", &body)
}

/// A home that adopts and enables the dog, with `record` as its
/// `[daemon.dog_grants.grantdog]` body when given.
#[cfg(unix)]
fn home_with(dir: &TempDir, dog: &Path, record: Option<&str>) -> PathBuf {
    let home = dir.path().to_path_buf();
    let record = record.map_or_else(String::new, |body| {
        format!("\n[daemon.dog_grants.grantdog]\n{body}\n")
    });
    std::fs::write(
        home.join("shep.toml"),
        format!(
            "[daemon]\nenabled_dogs = [\"grantdog\"]\n\n[daemon.adopted_dogs]\ngrantdog = \"{}\"\n{record}",
            dog.display()
        ),
    )
    .unwrap();
    home
}

fn shep_toml(home: &Path) -> String {
    std::fs::read_to_string(home.join("shep.toml")).unwrap()
}

/// The `access` of `grantdog` in `shep --format json dogs`.
#[cfg(unix)]
fn access_of_grantdog(home: &Path) -> serde_json::Value {
    let output = shep(home)
        .args(["--format", "json", "dogs"])
        .output()
        .unwrap();
    assert_success(&output);
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    envelope["data"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == "grantdog"))
        .unwrap_or_else(|| panic!("no grantdog row in {envelope}"))["access"]
        .clone()
}

#[cfg(unix)]
#[test]
fn a_manifest_dog_is_granted_its_list_once_and_marked_not_reviewed() {
    let dir = tempfile::tempdir().unwrap();
    let dog = write_dog(&dir, Some(&["flock.read"]));
    let home = home_with(&dir, &dog, None);
    let mut guard = DaemonGuard::default();
    let mustered = shep(&home).arg("muster").output().unwrap();
    guard.adopt_home(&home);
    assert_success(&mustered);

    let written = shep_toml(&home);
    assert!(
        written.contains("granted = [\"flock.read\"]") && written.contains("reviewed = false"),
        "{written}"
    );
    assert_eq!(
        access_of_grantdog(&home),
        serde_json::json!({"undeclared": false, "not_reviewed": true, "waiting": []})
    );

    let enabled = shep(&home)
        .args(["enable", "--yes", "grantdog"])
        .output()
        .unwrap();
    assert_success(&enabled);
    assert!(
        !shep_toml(&home).contains("reviewed"),
        "{}",
        shep_toml(&home)
    );
    assert_eq!(
        access_of_grantdog(&home),
        serde_json::json!({"undeclared": false, "not_reviewed": false, "waiting": []})
    );

    graceful_kill(&home);
}

#[cfg(unix)]
#[test]
fn a_dog_with_no_manifest_keeps_working_and_is_marked_undeclared() {
    let dir = tempfile::tempdir().unwrap();
    let dog = write_dog(&dir, None);
    let home = home_with(&dir, &dog, None);
    let mut guard = DaemonGuard::default();
    let mustered = shep(&home).arg("muster").output().unwrap();
    guard.adopt_home(&home);
    assert_success(&mustered);

    assert!(
        !shep_toml(&home).contains("dog_grants"),
        "{}",
        shep_toml(&home)
    );
    assert_eq!(
        access_of_grantdog(&home),
        serde_json::json!({"undeclared": true, "not_reviewed": false, "waiting": []})
    );
    guard.adopt_dog_pid(wait_for_dog_pid(&home, "grantdog"));

    graceful_kill(&home);
}

#[cfg(unix)]
#[test]
fn a_rebuilt_dog_asking_for_more_keeps_its_old_grants_until_approved() {
    let dir = tempfile::tempdir().unwrap();
    let dog = write_dog(&dir, Some(&["flock.read", "logs.read"]));
    let home = home_with(&dir, &dog, Some("granted = [\"flock.read\"]"));
    let mut guard = DaemonGuard::default();
    let mustered = shep(&home).arg("muster").output().unwrap();
    guard.adopt_home(&home);
    assert_success(&mustered);

    let written = shep_toml(&home);
    assert!(
        written.contains("granted = [\"flock.read\"]")
            && written.contains("waiting = [\"logs.read\"]"),
        "{written}"
    );
    assert_eq!(
        access_of_grantdog(&home),
        serde_json::json!({"undeclared": false, "not_reviewed": false, "waiting": ["logs.read"]})
    );

    let enabled = shep(&home)
        .args(["enable", "--yes", "grantdog"])
        .output()
        .unwrap();
    assert_success(&enabled);
    let written = shep_toml(&home);
    assert!(
        written.contains("granted = [\"flock.read\", \"logs.read\"]")
            && !written.contains("waiting"),
        "{written}"
    );

    graceful_kill(&home);
}
