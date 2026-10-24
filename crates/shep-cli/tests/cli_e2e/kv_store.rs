//! The key/value store through the real binary: `shep kv get`, `shep kv set`
//! and `shep kv unset`, and the hidden `shep unset` that still reaches it.
//!
//! `shep get` and `shep set` are not part of this file: they read and write a
//! sheep's or a dog's settings, and `settings_get.rs` and `settings_set.rs` are
//! where those are driven.
//!
//! None of these cases starts a shepherd: the store is a file under
//! `$SHEP_HOME`, and provisioning is exactly the moment nothing is running.

use super::*;

#[cfg(unix)]
/// The whole store through `shep kv`, with no shepherd anywhere: provisioning
/// happens when nothing is running. Also checks the `0600` mode
/// `shep_core::kv` documents, on the store the first `shep kv set` creates.
#[test]
fn the_kv_store_works_through_shep_kv_with_no_shepherd_running() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    let set1 = shep(home)
        .arg("kv")
        .arg("set")
        .arg("bark.cooldown")
        .arg("30s")
        .output()
        .unwrap();
    assert_success(&set1);
    assert!(
        !home.join("run").join("shep.sock").exists(),
        "`shep kv set` must never autostart a shepherd"
    );

    let mode = std::fs::metadata(home.join("kv.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "{mode:o}");

    let get1 = shep(home)
        .arg("kv")
        .arg("get")
        .arg("bark.cooldown")
        .output()
        .unwrap();
    assert_success(&get1);
    assert!(
        String::from_utf8_lossy(&get1.stdout).contains("30s"),
        "{}",
        String::from_utf8_lossy(&get1.stdout)
    );

    let missing = shep(home)
        .arg("kv")
        .arg("get")
        .arg("missing")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(3), "NotFound; {missing:?}");

    let set2 = shep(home)
        .arg("kv")
        .arg("set")
        .arg("metrics_port")
        .arg("9615")
        .output()
        .unwrap();
    assert_success(&set2);

    let both = shep(home).arg("kv").arg("get").output().unwrap();
    assert_success(&both);
    let both_text = String::from_utf8_lossy(&both.stdout);
    assert!(both_text.contains("bark.cooldown"), "{both_text}");
    assert!(both_text.contains("metrics_port"), "{both_text}");

    let unset1 = shep(home)
        .arg("kv")
        .arg("unset")
        .arg("bark.cooldown")
        .output()
        .unwrap();
    assert_success(&unset1);

    let gone = shep(home)
        .arg("kv")
        .arg("get")
        .arg("bark.cooldown")
        .output()
        .unwrap();
    assert_eq!(gone.status.code(), Some(3), "NotFound; {gone:?}");

    let unset_all = shep(home)
        .arg("kv")
        .arg("unset")
        .arg("--all")
        .output()
        .unwrap();
    assert_success(&unset_all);

    let empty = shep(home).arg("kv").arg("get").output().unwrap();
    assert_success(&empty);
    let empty_text = String::from_utf8_lossy(&empty.stdout);
    assert!(
        !empty_text.contains("metrics_port"),
        "store must be empty after `shep kv unset --all`: {empty_text}"
    );

    let bad_key = shep(home)
        .arg("kv")
        .arg("set")
        .arg("bad key")
        .arg("x")
        .output()
        .unwrap();
    assert_eq!(bad_key.status.code(), Some(2), "usage; {bad_key:?}");
}

/// `data` is an array of `{key, value}` objects, never a JSON map keyed by
/// name. An absent key and a key outside the grammar surface as this
/// envelope's error half.
#[test]
fn kv_json_envelope_is_an_array_with_the_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    shep(home)
        .arg("kv")
        .arg("set")
        .arg("a")
        .arg("1")
        .output()
        .unwrap();
    shep(home)
        .arg("kv")
        .arg("set")
        .arg("b")
        .arg("2")
        .output()
        .unwrap();

    let get_all = shep(home)
        .arg("--format")
        .arg("json")
        .arg("kv")
        .arg("get")
        .output()
        .unwrap();
    assert_success(&get_all);
    let envelope: serde_json::Value = serde_json::from_slice(&get_all.stdout).unwrap();
    assert!(envelope["data"].is_array(), "{envelope}");
    assert_eq!(envelope["data"].as_array().unwrap().len(), 2, "{envelope}");
    assert_eq!(envelope["schema_version"], 1, "{envelope}");
    assert_eq!(envelope["command"], "kv", "{envelope}");

    let missing = shep(home)
        .arg("--format")
        .arg("json")
        .arg("kv")
        .arg("get")
        .arg("ghost")
        .output()
        .unwrap();
    assert_json_error(&missing, 3, "not_found");

    let bad_key = shep(home)
        .arg("--format")
        .arg("json")
        .arg("kv")
        .arg("set")
        .arg("bad key")
        .arg("x")
        .output()
        .unwrap();
    assert_json_error(&bad_key, 2, "usage");
}

/// The old verb is hidden, not gone. It keeps its exit code and its stdout
/// answer, and says where it moved on stderr, in the format an operator at a
/// terminal reads. `shep get` and `shep set` are absent here: they are the
/// settings read and write now (`settings_get.rs`, `settings_set.rs`), and
/// `shep kv get` and `shep kv set` are the store's spellings.
#[test]
fn the_hidden_store_verb_still_works_and_names_where_it_moved() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    let seed = shep(home)
        .arg("kv")
        .arg("set")
        .arg("bark.cooldown")
        .arg("30s")
        .output()
        .unwrap();
    assert_success(&seed);

    let unset = shep(home)
        .arg("unset")
        .arg("bark.cooldown")
        .output()
        .unwrap();
    assert_success(&unset);
    assert!(
        String::from_utf8_lossy(&unset.stdout).contains("REMOVED"),
        "{}",
        String::from_utf8_lossy(&unset.stdout)
    );
    assert!(
        String::from_utf8_lossy(&unset.stderr).contains("`shep unset` moved to `shep kv unset`"),
        "{}",
        String::from_utf8_lossy(&unset.stderr)
    );
}

/// Under `--format json` a hidden alias prints one object per invocation: its
/// own envelope on stdout, and on failure its error envelope on stderr. The
/// note is a terminal reader's, and a second object on the same stream is what
/// a consumer merging the two streams chokes on.
#[test]
fn a_json_alias_run_prints_one_object_and_no_note() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    let seed = shep(home)
        .arg("kv")
        .arg("set")
        .arg("bark.cooldown")
        .arg("30s")
        .output()
        .unwrap();
    assert_success(&seed);

    let run = shep(home)
        .arg("--format")
        .arg("json")
        .arg("unset")
        .arg("bark.cooldown")
        .output()
        .unwrap();
    assert_success(&run);
    let envelope: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(envelope["command"], "unset", "{envelope}");
    assert!(
        run.stderr.is_empty(),
        "a JSON run prints its own envelope and nothing else: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    // `assert_json_error` parses stderr whole, so a notice riding in front of
    // the failure fails here rather than passing on a substring.
    let bad_key = shep(home)
        .arg("--format")
        .arg("json")
        .arg("unset")
        .arg("not a key")
        .output()
        .unwrap();
    assert_json_error(&bad_key, 2, "usage");

    let missing = shep(home)
        .arg("--format")
        .arg("json")
        .arg("kv")
        .arg("get")
        .arg("nope")
        .output()
        .unwrap();
    assert_json_error(&missing, 3, "not_found");
}

/// Both spellings reach one store. An operator mid-migration removes with the
/// verb they know and reads with the new one.
#[test]
fn the_two_spellings_share_one_store() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    let seed = shep(home)
        .arg("kv")
        .arg("set")
        .arg("feature.flag")
        .arg("on")
        .output()
        .unwrap();
    assert_success(&seed);

    let gone = shep(home)
        .arg("unset")
        .arg("feature.flag")
        .output()
        .unwrap();
    assert_success(&gone);

    let missing = shep(home)
        .arg("kv")
        .arg("get")
        .arg("feature.flag")
        .output()
        .unwrap();
    assert_eq!(missing.status.code(), Some(3), "NotFound; {missing:?}");
}

/// Bare `shep kv` refuses rather than guessing which of its three verbs was
/// meant, and writes no envelope to stdout while doing it. `shep kv --help` is
/// where an operator meets the three.
#[test]
fn bare_kv_refuses_and_its_help_names_the_three() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();

    let bare = shep(home).arg("kv").output().unwrap();
    assert_eq!(bare.status.code(), Some(2), "{bare:?}");
    assert!(
        bare.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&bare.stdout)
    );

    let help = shep(home).arg("kv").arg("--help").output().unwrap();
    assert_success(&help);
    let help_text = String::from_utf8_lossy(&help.stdout);
    let commands = help_text
        .split_once("Commands:")
        .unwrap_or_else(|| panic!("`shep kv --help` lists no commands: {help_text}"))
        .1;
    for verb in ["get", "set", "unset"] {
        assert!(
            commands.lines().any(|line| line
                .split_whitespace()
                .next()
                .is_some_and(|first| first == verb)),
            "`shep kv --help` does not list `{verb}`: {help_text}"
        );
    }
}

/// Two real processes, not two threads sharing one open-file-description
/// table: only separate processes contend for `kv.json.lock`'s `flock(2)`.
/// The barrier matters: without it one writer can finish its whole batch
/// before the other starts, racing nothing.
#[test]
fn two_real_shep_processes_writing_concurrently_lose_no_keys() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_path_buf();
    const PER_WRITER: usize = 15;

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let (finished, racers) = std::sync::mpsc::channel();
    for writer in 0..2 {
        let home = home.clone();
        let barrier = std::sync::Arc::clone(&barrier);
        let finished = finished.clone();
        std::thread::spawn(move || {
            barrier.wait(); // both writers start their first `shep kv set` together
            for n in 0..PER_WRITER {
                let key = format!("writer{writer}.k{n}");
                let output = shep(&home)
                    .arg("kv")
                    .arg("set")
                    .arg(&key)
                    .arg("v")
                    .output()
                    .unwrap();
                // A closed receiver means the case already gave up on this
                // writer and failed; there is no one left to report to.
                let _ = finished.send((writer, key, output));
            }
        });
    }
    drop(finished); // the writer threads hold the only senders that matter

    for _ in 0..(PER_WRITER * 2) {
        let (writer, key, output) = racers
            .recv_timeout(RACER_DEADLINE)
            .expect("a writer never came back; see RACER_DEADLINE");
        assert!(
            output.status.success(),
            "writer {writer}, key {key}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let list = shep(&home)
        .arg("--format")
        .arg("json")
        .arg("kv")
        .arg("get")
        .output()
        .unwrap();
    assert_success(&list);
    let envelope: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    let data = envelope["data"]
        .as_array()
        .unwrap_or_else(|| panic!("get data must be an array: {envelope}"));
    assert_eq!(
        data.len(),
        PER_WRITER * 2,
        "two concurrent shep kv set processes must not lose each other's keys: {envelope}"
    );
}
