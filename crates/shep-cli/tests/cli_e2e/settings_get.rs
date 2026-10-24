//! `shep get` against a live shepherd: a sheep's config, a dog table inside a
//! sheep, a dog's own section of `dogs.toml`, and the names that are none of
//! these.
//!
//! One case drives one flock, because the cost here is the daemon and the
//! sheep, not the reads. Every assertion is on the exact text a `shep get`
//! prints, since the claim is that the answer is the TOML the address names
//! and can be pasted back into the file it came from.

use super::*;

/// `dogs.toml` for the dog half of the case: a scalar, a sub-table with a
/// comment of its own, and nothing else.
const DOGS_TOML: &str = "\
# kelpie reads the repo it was adopted into
[kelpie]
repo = \"/srv/project\"

# what it may fetch
[kelpie.worker]
allowed_domains = [\"github.com\"]
timeout = 30

# a key spelled with a dot in it, quoted in the file
[kelpie.\"api.token\"]
\"auth.key\" = 42
";

/// The whole of `shep get` over one live flock.
#[test]
fn shep_get_reads_a_sheeps_settings_and_a_dogs_own_section() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let flockfile = write_flockfile(
        &dir,
        &format!(
            "[[app]]\nname = \"api\"\nscript = '{}'\nmax_memory = \"1G\"\n\n\
             [app.dogs.kelpie.git]\ncheckout = \"/srv/api\"\nmerging = \"ff\"\n\n\
             [app.dogs.kelpie.options]\n",
            script.display()
        ),
    );
    std::fs::write(home.join("dogs.toml"), DOGS_TOML).unwrap();

    let mut guard = DaemonGuard::default();
    let boot = shep(home).arg("start").arg(&flockfile).output().unwrap();
    guard.adopt_home(home);
    assert_success(&boot);
    poll_flock(home, |info| info["status"] == "online");

    // A name alone prints the config, dog tables and all, under the headers a
    // Flockfile spells for them.
    let whole = shep(home).arg("get").arg("api").output().unwrap();
    assert_success(&whole);
    let text = String::from_utf8_lossy(&whole.stdout);
    assert!(text.contains("max_memory = \"1G\""), "{text}");
    assert!(text.contains("[dogs.kelpie.git]"), "{text}");
    assert!(text.contains("checkout = \"/srv/api\""), "{text}");

    // One field is one line, spelled as the Flockfile spelled it.
    let field = shep(home)
        .arg("get")
        .arg("api.max_memory")
        .output()
        .unwrap();
    assert_success(&field);
    assert_eq!(
        String::from_utf8_lossy(&field.stdout),
        "max_memory = \"1G\"\n"
    );

    // A dog table inside a sheep carries the header the Flockfile gave it, so
    // the answer pastes back into that Flockfile.
    let git = shep(home)
        .arg("get")
        .arg("api.dogs.kelpie.git")
        .output()
        .unwrap();
    assert_success(&git);
    assert_eq!(
        String::from_utf8_lossy(&git.stdout),
        "[dogs.kelpie.git]\ncheckout = \"/srv/api\"\nmerging = \"ff\"\n"
    );

    // A dog's own section, re-rooted under its name, keeps the comment the
    // operator wrote above its sub-table.
    let dog = shep(home).arg("get").arg("kelpie").output().unwrap();
    assert_success(&dog);
    let text = String::from_utf8_lossy(&dog.stdout);
    assert!(text.starts_with("[kelpie]\n"), "{text}");
    assert!(text.contains("# what it may fetch"), "{text}");

    // `kelpie.worker` is the `worker` table of the dog `kelpie`: `dogs.toml`
    // spells that table as a header path, not as a name.
    let worker = shep(home).arg("get").arg("kelpie.worker").output().unwrap();
    assert_success(&worker);
    assert_eq!(
        String::from_utf8_lossy(&worker.stdout),
        "[kelpie.worker]\nallowed_domains = [\"github.com\"]\ntimeout = 30\n"
    );

    // A name no sheep is and no dog has a section for is `not_found`, and the
    // sentence names where the key/value store `shep get` used to read went.
    let missing = shep(home).arg("get").arg("nosuch.key").output().unwrap();
    assert_eq!(missing.status.code(), Some(3), "NotFound; {missing:?}");
    let stderr = String::from_utf8_lossy(&missing.stderr);
    assert!(stderr.contains("shep kv get"), "{stderr}");

    // A dog the flock runs with nothing written for it is the other answer:
    // the name is right, and every setting it has is a default. The dog named
    // is `bark`, the one the flock carries, not the `bark.interval` the
    // operator typed, and `shep enable` is aimed at that name.
    let silent = shep(home).arg("get").arg("bark.interval").output().unwrap();
    assert_eq!(silent.status.code(), Some(3), "NotFound; {silent:?}");
    let stderr = String::from_utf8_lossy(&silent.stderr);
    assert!(
        stderr.contains("`bark` is a dog with nothing set"),
        "{stderr}"
    );
    assert!(stderr.contains("default"), "{stderr}");
    assert!(stderr.contains("shep enable bark`"), "{stderr}");
    assert!(!stderr.contains("shep enable bark.interval`"), "{stderr}");

    // A path that ends at a value prints that value under the header of the
    // table that holds it, which is what `shep get --help` promises.
    let leaf = shep(home)
        .arg("get")
        .arg("kelpie.worker.timeout")
        .output()
        .unwrap();
    assert_success(&leaf);
    assert_eq!(
        String::from_utf8_lossy(&leaf.stdout),
        "[kelpie.worker]\ntimeout = 30\n"
    );

    // A key spelled with a dot in it, quoted in `dogs.toml`, is one key on
    // the way down too: the address names it segment by segment and the walk
    // descends by the key it matched.
    let quoted = shep(home)
        .arg("get")
        .arg("kelpie.api.token.auth.key")
        .output()
        .unwrap();
    assert_success(&quoted);
    assert_eq!(
        String::from_utf8_lossy(&quoted.stdout),
        "[kelpie.\"api.token\"]\n\"auth.key\" = 42\n"
    );

    // `[app.dogs.kelpie.options]` with no entries is a setting the operator
    // wrote, and its header is the only way to say it is there: a dog's
    // settings are opaque to shep, so an empty table is not a missing one.
    let empty = shep(home)
        .arg("get")
        .arg("api.dogs.kelpie.options")
        .output()
        .unwrap();
    assert_success(&empty);
    assert_eq!(
        String::from_utf8_lossy(&empty.stdout),
        "[dogs.kelpie.options]\n"
    );
    let whole_text = String::from_utf8_lossy(&whole.stdout);
    assert!(whole_text.contains("[dogs.kelpie.options]"), "{whole_text}");

    let absent = shep(home)
        .arg("get")
        .arg("api.dogs.kelpie.git.remote")
        .output()
        .unwrap();
    assert_eq!(absent.status.code(), Some(3), "NotFound; {absent:?}");

    // `env` values never reach a client, so there is nothing here to print.
    let env = shep(home).arg("get").arg("api.env").output().unwrap();
    assert_eq!(env.status.code(), Some(4), "InvalidConfig; {env:?}");
    let stderr = String::from_utf8_lossy(&env.stderr);
    assert!(stderr.contains("shep import env"), "{stderr}");

    // `--format json` is one envelope, and the document rides in it as text.
    let json = shep(home)
        .arg("--format")
        .arg("json")
        .arg("get")
        .arg("kelpie.worker")
        .output()
        .unwrap();
    assert_success(&json);
    let envelope: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(envelope["command"], "get");
    assert_eq!(envelope["data"]["kind"], "dog");
    assert_eq!(envelope["data"]["name"], "kelpie");
    assert_eq!(envelope["data"]["path"], "worker");
    assert_eq!(
        envelope["data"]["toml"],
        "[kelpie.worker]\nallowed_domains = [\"github.com\"]\ntimeout = 30\n"
    );

    // Bare `shep get` no longer lists the store.
    let bare = shep(home).arg("get").output().unwrap();
    assert_eq!(bare.status.code(), Some(2), "Usage; {bare:?}");

    graceful_kill(home);
}
