//! `shep set` against a live shepherd: a sheep's own field, a key inside a
//! sheep's dog table, and a key of a dog's own section, each written through
//! the request the shepherd already answers.
//!
//! One case drives one flock, because the cost here is the daemon and the
//! sheep, not the writes. Every assertion is on the exact text a `shep set`
//! prints beside the value the flock holds afterwards, since the claim is that
//! an operator sees every key a write touches before it touches them.

use super::*;

/// `dogs.toml` for the dog half: a comment above the header, one key the
/// operator wrote by hand, and nothing else. `bark` is a built-in dog, so it is
/// a dog this shepherd knows even with nothing running configured, and it is a
/// dog that publishes a schema of its own.
const DOGS_TOML: &str = "\
# how often bark re-checks delivery
[bark]
poll = \"30s\"
";

/// The whole of `shep set` over one live flock.
#[test]
fn shep_set_writes_a_sheeps_field_a_dog_table_key_and_a_dogs_own_key() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let flockfile = write_flockfile(
        &dir,
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

    // A sheep's own field: the change list names the key with its old value
    // and its new one, and the flock holds the new one afterwards.
    let field = shep(home)
        .arg("set")
        .arg("api.max_memory")
        .arg("512M")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&field);
    let text = String::from_utf8_lossy(&field.stdout);
    assert!(text.contains("max_memory"), "{text}");
    assert!(text.contains("changed"), "{text}");
    assert!(text.contains("1G"), "{text}");
    assert!(text.contains("512M"), "{text}");
    assert!(text.contains("wrote 1 change onto `api`"), "{text}");

    let read = shep(home)
        .arg("get")
        .arg("api.max_memory")
        .output()
        .unwrap();
    assert_success(&read);
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "max_memory = \"512M\"\n"
    );

    // A key inside a sheep's dog table: the one key moves and the key beside
    // it stays, because the write is the merged table.
    let nested = shep(home)
        .arg("set")
        .arg("api.dogs.kelpie.git.merging")
        .arg("rebase")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&nested);
    let text = String::from_utf8_lossy(&nested.stdout);
    assert!(text.contains("merging"), "{text}");
    assert!(text.contains("ff"), "{text}");
    assert!(text.contains("rebase"), "{text}");

    let read = shep(home)
        .arg("get")
        .arg("api.dogs.kelpie.git")
        .output()
        .unwrap();
    assert_success(&read);
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "[dogs.kelpie.git]\ncheckout = \"/srv/api\"\nmerging = \"rebase\"\n"
    );

    // A dog's own key, in a section the operator wrote by hand: the comment
    // above the header and the key beside the one written both survive, because
    // the write edits the text the shepherd answered rather than replacing it
    // with a rendered map.
    let dog = shep(home)
        .arg("set")
        .arg("bark.history_bytes")
        .arg("1048576")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&dog);
    let text = String::from_utf8_lossy(&dog.stdout);
    assert!(text.contains("history_bytes"), "{text}");
    assert!(text.contains("added"), "{text}");

    let read = shep(home).arg("get").arg("bark").output().unwrap();
    assert_success(&read);
    let text = String::from_utf8_lossy(&read.stdout);
    assert!(text.starts_with("[bark]\n"), "{text}");
    assert!(text.contains("poll = \"30s\""), "{text}");
    assert!(text.contains("history_bytes = 1048576"), "{text}");

    // The file itself is what the claim is about: the comment above the header
    // and the key beside the one written both survive, because the write edits
    // the text the shepherd answered rather than replacing it with a rendered
    // map. `shep get` re-roots a section and clears the header's own decor, so
    // that comment is only visible here.
    let file = std::fs::read_to_string(home.join("dogs.toml")).unwrap();
    assert!(
        file.contains("# how often bark re-checks delivery"),
        "{file}"
    );
    assert!(file.contains("poll = \"30s\""), "{file}");
    assert!(file.contains("history_bytes = 1048576"), "{file}");

    // A value the dog's own schema turns down is refused with the key path it
    // sits at, and the section is left exactly as it was.
    let refused = shep(home)
        .arg("set")
        .arg("bark.history_bytes")
        .arg("\"not-a-number\"")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(4), "InvalidConfig; {refused:?}");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("nothing was written"), "{stderr}");
    assert!(stderr.contains("`bark.history_bytes`"), "{stderr}");

    let read = shep(home)
        .arg("get")
        .arg("bark.history_bytes")
        .output()
        .unwrap();
    assert_success(&read);
    // `shep get` prints a value under the header of the table that holds it.
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "[bark]\nhistory_bytes = 1048576\n"
    );

    // `--dry-run` prints the same list and writes nothing.
    let dry = shep(home)
        .arg("set")
        .arg("api.max_memory")
        .arg("2G")
        .arg("--dry-run")
        .output()
        .unwrap();
    assert_success(&dry);
    let text = String::from_utf8_lossy(&dry.stdout);
    assert!(text.contains("512M"), "{text}");
    assert!(text.contains("2G"), "{text}");
    assert!(text.contains("nothing was written"), "{text}");

    let read = shep(home)
        .arg("get")
        .arg("api.max_memory")
        .output()
        .unwrap();
    assert_success(&read);
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "max_memory = \"512M\"\n"
    );

    // Writing without asking needs a terminal or `--yes`. `Command::output`
    // gives this process no terminal on stdin, which is exactly the shape a
    // script runs in, and a run that cannot ask refuses instead of guessing.
    let unasked = shep(home)
        .arg("set")
        .arg("api.max_memory")
        .arg("8G")
        .output()
        .unwrap();
    assert_eq!(unasked.status.code(), Some(2), "Usage; {unasked:?}");
    let stderr = String::from_utf8_lossy(&unasked.stderr);
    assert!(stderr.contains("no terminal"), "{stderr}");
    assert!(stderr.contains("--yes"), "{stderr}");

    let read = shep(home)
        .arg("get")
        .arg("api.max_memory")
        .output()
        .unwrap();
    assert_success(&read);
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "max_memory = \"512M\"\n"
    );

    // The keys this verb never writes, refused with `copy`'s reasons: `env`
    // names the door that writes env, `name` and `instances` name the verbs
    // that move a sheep and an instance count, and the whole `dogs` map names a
    // dog.
    for (address, expect) in [
        ("api.env", "shep import env"),
        ("api.name", "a name change makes a different sheep"),
        ("api.instances", "shep stock"),
        ("api.dogs", "name a dog"),
    ] {
        let run = shep(home)
            .arg("set")
            .arg(address)
            .arg("x")
            .arg("--yes")
            .output()
            .unwrap();
        assert_eq!(run.status.code(), Some(4), "{address}: {run:?}");
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(stderr.contains(expect), "{address}: {stderr}");
    }

    // The key/value store this verb used to write is not this verb's any more:
    // the name is refused, the sentence names where the store lives, and the
    // store is still empty.
    let store = shep(home)
        .arg("set")
        .arg("feature.flag")
        .arg("on")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(store.status.code(), Some(3), "NotFound; {store:?}");
    let stderr = String::from_utf8_lossy(&store.stderr);
    assert!(
        stderr.contains("no sheep or dog named `feature.flag`"),
        "{stderr}"
    );
    assert!(stderr.contains("shep kv set"), "{stderr}");

    let read = shep(home)
        .arg("kv")
        .arg("get")
        .arg("feature.flag")
        .output()
        .unwrap();
    assert_eq!(read.status.code(), Some(3), "NotFound; {read:?}");

    // `--format json` is one envelope, carrying the change list beside the
    // run's own verdict, and nothing else on either stream.
    let json = shep(home)
        .arg("--format")
        .arg("json")
        .arg("set")
        .arg("api.dogs.kelpie.git.remote")
        .arg("upstream")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&json);
    assert!(
        json.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&json.stderr)
    );
    let envelope: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(envelope["command"], "set", "{envelope}");
    assert_eq!(envelope["schema_version"], 1, "{envelope}");
    assert_eq!(envelope["data"]["kind"], "sheep", "{envelope}");
    assert_eq!(envelope["data"]["name"], "api", "{envelope}");
    assert_eq!(
        envelope["data"]["path"], "dogs.kelpie.git.remote",
        "{envelope}"
    );
    assert_eq!(envelope["data"]["written"], true, "{envelope}");
    assert_eq!(envelope["data"]["dry_run"], false, "{envelope}");
    let changes = envelope["data"]["changes"]
        .as_array()
        .unwrap_or_else(|| panic!("changes must be an array: {envelope}"));
    assert_eq!(changes.len(), 1, "{envelope}");
    assert_eq!(changes[0]["key"], "remote", "{envelope}");
    assert_eq!(changes[0]["change"], "added", "{envelope}");
    assert!(changes[0]["old"].is_null(), "{envelope}");
    assert_eq!(changes[0]["new"], "upstream", "{envelope}");

    // A value already held is no write at all: no question, no request, and a
    // change list with nothing in it.
    let same = shep(home)
        .arg("--format")
        .arg("json")
        .arg("set")
        .arg("api.max_memory")
        .arg("512M")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&same);
    let envelope: serde_json::Value = serde_json::from_slice(&same.stdout).unwrap();
    assert_eq!(envelope["data"]["written"], false, "{envelope}");
    assert_eq!(
        envelope["data"]["changes"],
        serde_json::json!([]),
        "{envelope}"
    );

    // `shep set` is a verb `--help` lists, in the group `shep get` is in.
    let help = shep(home).arg("--help").output().unwrap();
    assert_success(&help);
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(
        help_text
            .lines()
            .any(|line| line.starts_with("The shepherd") && line.contains(" set ")),
        "{help_text}"
    );

    graceful_kill(home);
}

/// `shep set` writes a dog's own key into a section that does not exist yet,
/// for a dog this home has never been given a section for.
#[test]
fn shep_set_creates_a_dogs_section_when_it_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let flockfile = write_flockfile(
        &dir,
        &format!("[[app]]\nname = \"api\"\nscript = '{}'\n", script.display()),
    );

    let mut guard = DaemonGuard::default();
    let boot = shep(home).arg("start").arg(&flockfile).output().unwrap();
    guard.adopt_home(home);
    assert_success(&boot);
    poll_flock(home, |info| info["status"] == "online");
    assert!(
        !home.join("dogs.toml").exists(),
        "the case is a home with no `dogs.toml` at all"
    );

    let written = shep(home)
        .arg("set")
        .arg("metrics.otlp_timeout")
        .arg("\"12s\"")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&written);
    let text = String::from_utf8_lossy(&written.stdout);
    assert!(text.contains("wrote 1 change onto `metrics`"), "{text}");

    let read = shep(home)
        .arg("get")
        .arg("metrics.otlp_timeout")
        .output()
        .unwrap();
    assert_success(&read);
    assert_eq!(
        String::from_utf8_lossy(&read.stdout),
        "[metrics]\notlp_timeout = \"12s\"\n"
    );

    graceful_kill(home);
}

/// A `dogs.toml` section for a name this shepherd knows no dog by is refused
/// before anything is sent, which is the daemon's own guard in a sentence that
/// names the remedy.
#[test]
fn shep_set_refuses_a_section_of_a_dog_nothing_knows() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let flockfile = write_flockfile(
        &dir,
        &format!("[[app]]\nname = \"api\"\nscript = '{}'\n", script.display()),
    );
    let stale = "\
# left behind by a dog that was removed
[ghost]
timeout = 30
";
    std::fs::write(home.join("dogs.toml"), stale).unwrap();

    let mut guard = DaemonGuard::default();
    let boot = shep(home).arg("start").arg(&flockfile).output().unwrap();
    guard.adopt_home(home);
    assert_success(&boot);
    poll_flock(home, |info| info["status"] == "online");

    let refused = shep(home)
        .arg("set")
        .arg("ghost.timeout")
        .arg("5")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    let text = String::from_utf8_lossy(&refused.stderr);
    assert!(text.contains("nothing was written"), "{text}");
    assert!(text.contains("`[ghost]` section"), "{text}");
    assert!(
        text.contains("`shep adopt` or `shep enable`"),
        "{text}: the remedy is the daemon's own"
    );
    assert!(
        !text.contains("shep kv set"),
        "{text}: this name is a dog's, not a key/value key"
    );

    assert_eq!(
        std::fs::read_to_string(home.join("dogs.toml")).unwrap(),
        stale,
        "a refusal writes nothing"
    );

    graceful_kill(home);
}
