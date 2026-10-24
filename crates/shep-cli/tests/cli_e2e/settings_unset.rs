//! `shep unset` against a live shepherd: a sheep's override dropped so the
//! Flockfile's value returns, and a key removed from a dog's table or section
//! so the dog's default applies.
//!
//! One case drives one flock, because the cost here is the daemon and the
//! sheep. Every assertion is on the text `shep unset` prints beside what
//! `shep get` reads afterwards, since the claim is that the value read back is
//! the one the operator removed their own word for.

use super::*;

/// `dogs.toml` with a comment above the header and two keys, so a removal has a
/// neighbour to leave alone.
const DOGS_TOML: &str = "\
# how often bark re-checks delivery
[bark]
poll = \"30s\"
history_bytes = 1048576
";

/// Starts `api` with a Flockfile that declares `max_memory` and one dog table.
fn start_flock(dir: &tempfile::TempDir) -> DaemonGuard {
    let home = dir.path();
    let script = write_test_script(dir);
    let flockfile = write_flockfile(
        dir,
        &format!(
            "[[app]]\nname = \"api\"\nscript = '{}'\nmax_memory = \"1G\"\n\n\
             [app.dogs.kelpie.git]\ncheckout = \"/srv/api\"\nmerging = \"ff\"\n",
            script.display()
        ),
    );
    std::fs::write(home.join("dogs.toml"), DOGS_TOML).unwrap();
    let mut guard = DaemonGuard::default();
    let boot = shep(home).arg("start").arg(&flockfile).output().unwrap();
    guard.adopt_home(home);
    assert_success(&boot);
    poll_flock(home, |info| info["status"] == "online");
    guard
}

/// What `shep get <address>` prints.
fn read(home: &std::path::Path, address: &str) -> String {
    let out = shep(home).arg("get").arg(address).output().unwrap();
    assert_success(&out);
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Dropping an override returns the Flockfile's value, and the flags are
/// `shep set`'s.
#[test]
fn shep_unset_returns_the_flockfiles_value_to_a_field_and_asks_like_set() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let _guard = start_flock(&dir);

    let set = shep(home)
        .args(["set", "api.max_memory", "512M", "--yes"])
        .output()
        .unwrap();
    assert_success(&set);
    assert_eq!(read(home, "api.max_memory"), "max_memory = \"512M\"\n");

    // A run that cannot ask refuses, and nothing changes.
    let unasked = shep(home)
        .args(["unset", "api.max_memory"])
        .output()
        .unwrap();
    assert_eq!(unasked.status.code(), Some(2), "Usage; {unasked:?}");
    let stderr = String::from_utf8_lossy(&unasked.stderr);
    assert!(
        stderr.contains("`shep unset` asks before it writes"),
        "{stderr}"
    );
    assert_eq!(read(home, "api.max_memory"), "max_memory = \"512M\"\n");

    // `--dry-run` prints the removal and writes nothing.
    let dry = shep(home)
        .args(["unset", "api.max_memory", "--dry-run"])
        .output()
        .unwrap();
    assert_success(&dry);
    let text = String::from_utf8_lossy(&dry.stdout);
    assert!(text.contains("removed"), "{text}");
    assert!(text.contains("512M"), "{text}");
    assert!(text.contains("nothing was written"), "{text}");
    assert_eq!(read(home, "api.max_memory"), "max_memory = \"512M\"\n");

    let unset = shep(home)
        .args(["unset", "api.max_memory", "--yes"])
        .output()
        .unwrap();
    assert_success(&unset);
    let text = String::from_utf8_lossy(&unset.stdout);
    assert!(text.contains("removed"), "{text}");
    assert!(text.contains("wrote 1 change onto `api`"), "{text}");
    assert!(
        text.contains("`api.max_memory` holds \"1G\" again"),
        "{text}"
    );
    assert_eq!(read(home, "api.max_memory"), "max_memory = \"1G\"\n");

    // The field holds the Flockfile's value now, so there is no override to
    // drop and the run says so rather than reporting a write.
    let again = shep(home)
        .args(["unset", "api.max_memory", "--yes"])
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(4), "InvalidConfig; {again:?}");
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert!(stderr.contains("has no override to drop"), "{stderr}");

    // `env` is refused for the reason `shep set` refuses it.
    let env = shep(home)
        .args(["unset", "api.env", "--yes"])
        .output()
        .unwrap();
    assert_eq!(env.status.code(), Some(4), "InvalidConfig; {env:?}");
}

/// A key leaves a sheep's dog table or a dog's own section, and what sits
/// beside it stays.
#[test]
fn shep_unset_removes_a_dog_key_and_keeps_its_neighbours() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let _guard = start_flock(&dir);

    let nested = shep(home)
        .args(["unset", "api.dogs.kelpie.git.merging", "--yes"])
        .output()
        .unwrap();
    assert_success(&nested);
    let text = String::from_utf8_lossy(&nested.stdout);
    assert!(text.contains("merging"), "{text}");
    assert!(text.contains("removed"), "{text}");
    assert_eq!(
        read(home, "api.dogs.kelpie.git"),
        "[dogs.kelpie.git]\ncheckout = \"/srv/api\"\n"
    );

    let missing = shep(home)
        .args(["unset", "api.dogs.kelpie.git.merging", "--yes"])
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(4), "InvalidConfig; {missing:?}");
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("holds nothing to remove"), "{stderr}");

    let dog = shep(home)
        .args(["unset", "bark.poll", "--yes"])
        .output()
        .unwrap();
    assert_success(&dog);
    let file = std::fs::read_to_string(home.join("dogs.toml")).unwrap();
    assert!(!file.contains("poll"), "{file}");
    assert!(file.contains("history_bytes = 1048576"), "{file}");
    assert!(
        file.contains("# how often bark re-checks delivery"),
        "{file}"
    );

    // A whole section is not one key.
    let section = shep(home)
        .args(["unset", "bark", "--yes"])
        .output()
        .unwrap();
    assert_eq!(section.status.code(), Some(2), "Usage; {section:?}");
    let file = std::fs::read_to_string(home.join("dogs.toml")).unwrap();
    assert!(file.contains("history_bytes = 1048576"), "{file}");
}

/// Under `--format json` the run prints one envelope under `unset`.
#[test]
fn shep_unset_prints_one_json_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let _guard = start_flock(&dir);

    assert_success(
        &shep(home)
            .args(["set", "api.max_memory", "512M", "--yes"])
            .output()
            .unwrap(),
    );
    let out = shep(home)
        .args(["--format", "json", "unset", "api.max_memory", "--yes"])
        .output()
        .unwrap();
    assert_success(&out);
    let envelope: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(envelope["command"], "unset", "{envelope}");
    assert_eq!(envelope["data"]["written"], true, "{envelope}");
    assert_eq!(
        envelope["data"]["changes"][0]["change"], "removed",
        "{envelope}"
    );
}
