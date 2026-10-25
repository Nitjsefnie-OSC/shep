//! Real dogs answering real scrapes, across a real reload, and the bark
//! history. The key/value store has its own file, `kv_store.rs`.

use super::*;

#[cfg(unix)]
/// The setup the three cases below share: a `$SHEP_HOME` whose
/// `[dog.metrics]` bind sits on a fresh free port, one sheep named `web`
/// started and polled to `online`, and the metrics dog enabled with its pid
/// registered with `guard`. Returns the bind port, the online row, and the
/// sheep's script; what each case asserts after the start is its own.
#[track_caller]
fn start_web_with_metrics_dog(
    dir: &TempDir,
    guard: &mut DaemonGuard,
) -> (u16, serde_json::Value, PathBuf) {
    let home = dir.path();
    let port = free_port();
    write_shep_toml(
        dir,
        &format!("[dog.metrics]\nbind = \"127.0.0.1:{port}\"\n"),
    );
    let script = write_test_script(dir);

    let started = shep(home)
        .arg("start")
        .arg(&script)
        .arg("--name")
        .arg("web")
        .output()
        .unwrap();
    guard.adopt_home(home);
    assert_success(&started);

    let online = poll_flock(home, |info| info["status"] == "online");

    let enabled = shep(home)
        .arg("enable")
        .arg("--yes")
        .arg("metrics")
        .output()
        .unwrap();
    assert_success(&enabled);

    // Registered before the caller's first assertion: a case that hangs or
    // panics must not leak the grandchild the daemon just spawned.
    let dog_pid = wait_for_dog_pid(home, "metrics");
    guard.adopt_dog_pid(dog_pid);

    (port, online, script)
}

#[cfg(unix)]
/// The only tier that exec's `shep dog metrics`: every other scripts the
/// runner or fakes the client.
///
/// Four things fail here as a refused connection: the dog being spawned, its
/// reaching the socket from `$SHEP_HOME`, its fetching its own
/// `[dog.metrics]` section, and its bind.
#[test]
fn a_real_shepherd_runs_a_real_metrics_dog_that_answers_a_scrape() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut guard = DaemonGuard::default();

    let (port, online, _script) = start_web_with_metrics_dog(&dir, &mut guard);
    assert_eq!(
        online["status"], "online",
        "the sheep must reach online before the dog's own exposition has \
         anything real to name: {online}"
    );

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let body = poll_metrics(addr);
    assert!(
        body.contains("HTTP/1.1 200"),
        "the metrics dog must answer 200 at /metrics: {body}"
    );
    assert!(
        body.contains(r#"shep_sheep_status{sheep="web",id="2",fold="",status="online"} 1"#),
        "the exposition must name the sheep, online: {body}"
    );
    assert!(
        body.contains(r#"shep_dog_up{dog="metrics",source="built-in"} 1"#),
        "the dog must report itself up while it is the one serving the \
         scrape that says so: {body}"
    );

    graceful_kill(home);
}

#[cfg(unix)]
/// A real dog process, carried across a real `execve`, still able to talk to
/// the shepherd that replaced the one it handshook with.
///
/// A pid check cannot see the defect: a dog holding a dead socket reads as
/// healthy on every column a listing has. The decisive assertion is content, a
/// sheep started after the reload appearing in the exposition.
#[test]
fn a_carried_dog_answers_a_scrape_after_a_real_reload() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut guard = DaemonGuard::default();

    let (port, online, script) = start_web_with_metrics_dog(&dir, &mut guard);
    let sheep_pid = online["pid"].as_u64().expect("an online sheep has a pid");
    // The pid the helper already registered with `guard`, read back for the
    // carried-pid assertion below.
    let dog_pid = wait_for_dog_pid(home, "metrics");

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let before = poll_metrics(addr);
    assert!(
        before.contains("HTTP/1.1 200"),
        "the dog must be answering BEFORE the reload, or this case proves nothing: {before}"
    );

    let reloaded = shep(home).arg("daemon").arg("reload").output().unwrap();
    assert_success(&reloaded);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&reloaded.stdout),
        String::from_utf8_lossy(&reloaded.stderr)
    );
    // The exact sentence `handover::RefusedReason`'s `Display` ends with: the
    // stop arm restarts every dog from disk and would satisfy a looser probe.
    assert!(
        !text.contains("falls back to a stop-and-start"),
        "a flock with a dog in it is carried now, not refused: {text}"
    );
    assert!(
        !text.contains("cannot talk to this shepherd"),
        "the carried dog must reconnect rather than be reported stale: {text}"
    );
    assert!(
        !text.contains("cannot say whether it came back"),
        "the carried dog must answer inside the reload's own wait: {text}"
    );

    let after = poll_flock_listing(home, FLOCK_DEADLINE, |data| {
        let live = |name: &str| {
            data.as_array()
                .into_iter()
                .flatten()
                .any(|row| row["name"] == name && !row["pid"].is_null())
        };
        live("metrics") && live("web")
    });
    let rows = after.as_array().expect("flock data is an array");
    let dog_row = rows
        .iter()
        .find(|row| row["name"] == "metrics")
        .unwrap_or_else(|| panic!("the dog must still be registered: {after}"));
    let sheep_row = rows
        .iter()
        .find(|row| row["name"] == "web")
        .unwrap_or_else(|| panic!("the sheep must still be registered: {after}"));

    assert_eq!(
        dog_row["pid"].as_u64(),
        Some(u64::try_from(dog_pid.as_raw()).unwrap()),
        "the dog was restarted rather than carried: {after}"
    );
    assert_eq!(
        sheep_row["pid"].as_u64(),
        Some(sheep_pid),
        "the sheep was restarted rather than carried: {after}"
    );
    assert_eq!(
        dog_row["restarts"], 0,
        "the dog's restart count moved: {after}"
    );
    assert_eq!(
        sheep_row["restarts"], 0,
        "the sheep's restart count moved: {after}"
    );
    // JSON carries both populations in one undivided array, so the marker is
    // what keeps them apart here; the tables below are what an operator sees.
    assert_eq!(
        dog_row["dog"]["kind"], "built_in",
        "a carried dog that lost its marker is one `shep dogs` has lost: {after}"
    );
    assert!(
        sheep_row["dog"].is_null(),
        "a sheep must not pick a marker up on the way across: {after}"
    );

    let dogs = shep(home).arg("dogs").output().unwrap();
    assert_success(&dogs);
    assert!(
        String::from_utf8_lossy(&dogs.stdout).contains("metrics"),
        "`shep dogs` must still list the carried dog: {}",
        String::from_utf8_lossy(&dogs.stdout)
    );
    let flock = shep(home).arg("flock").output().unwrap();
    assert_success(&flock);
    let flock_text = String::from_utf8_lossy(&flock.stdout);
    let (sheep_table, _dogs_table) = flock_text
        .split_once("Dogs")
        .unwrap_or_else(|| panic!("`shep flock` prints a dogs section: {flock_text}"));
    assert!(
        !sheep_table.contains("metrics"),
        "a carried dog must not be listed beside the operator's own apps: {flock_text}"
    );

    // The decisive one. A sheep that did not exist when the predecessor was
    // running, named by the dog that is answering now.
    let fresh = shep(home)
        .arg("start")
        .arg(&script)
        .arg("--name")
        .arg("freshsheep")
        .output()
        .unwrap();
    assert_success(&fresh);
    let body = poll_metrics_containing(addr, r#"sheep="freshsheep""#);
    assert!(
        body.contains("HTTP/1.1 200"),
        "the carried dog must still answer a scrape after the exec: {body}"
    );
    assert!(
        body.contains(r#"sheep="freshsheep""#),
        "the exposition must name a sheep started AFTER the reload, which no cached reading \
         and no connection to the predecessor could produce: {body}"
    );

    // A successor rebuilds the roll from the blob, not from disk.
    // `spawn_enabled_dogs` never touches `FlockRegistry`, so a dog in the roll
    // would outlive the daemon and a later cold boot would restore `metrics`
    // as an ordinary unmarked sheep.
    let saved = shep(home)
        .arg("--format")
        .arg("json")
        .arg("save")
        .output()
        .unwrap();
    assert_success(&saved);
    let roll: serde_json::Value = serde_json::from_slice(&saved.stdout).unwrap();
    assert_eq!(
        roll["data"]["apps"], 2,
        "the roll holds the two sheep and no dog: {roll}"
    );

    graceful_kill(home);
}

#[cfg(unix)]
/// Table format, not JSON: `Format::Json`'s `flock` answer carries both
/// populations in one undivided array.
#[test]
fn dogs_and_flock_render_the_two_populations_the_right_way_round() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut guard = DaemonGuard::default();

    start_web_with_metrics_dog(&dir, &mut guard);

    let flock_table = String::from_utf8(shep(home).arg("flock").output().unwrap().stdout).unwrap();
    assert!(
        flock_table.contains("web"),
        "shep flock must still render the sheep: {flock_table}"
    );
    assert!(
        flock_table.contains("Dogs") && flock_table.contains("metrics"),
        "shep flock must render the dogs section beneath the sheep table: {flock_table}"
    );

    let dogs_table = String::from_utf8(shep(home).arg("dogs").output().unwrap().stdout).unwrap();
    assert!(
        dogs_table.contains("metrics"),
        "shep dogs must render the dog: {dogs_table}"
    );
    assert!(
        !dogs_table.contains("web"),
        "shep dogs must render nothing but dogs — not the sheep: {dogs_table}"
    );
    assert!(
        !dogs_table.contains("Dogs\n"),
        "shep dogs must not carry flock's own section header — it IS the \
         dogs table, not a listing with one embedded: {dogs_table}"
    );

    graceful_kill(home);
}

#[test]
fn barks_reads_the_history_with_no_shepherd_running() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let bark = shep_core::barks::Bark {
        at_ms: 1_700_000_000_000,
        rule: "watchdog".to_string(),
        subject: "web".to_string(),
        message: "restart budget exhausted".to_string(),
        sinks: vec![shep_core::barks::SinkOutcome {
            sink: "ops".to_string(),
            error: None,
        }],
    };
    shep_core::barks::append(
        &home.join("barks.jsonl"),
        &bark,
        shep_core::barks::DEFAULT_MAX_BYTES,
    )
    .unwrap();
    assert!(
        !home.join("run").join("shep.sock").exists(),
        "this case never starts a daemon at all"
    );

    let output = shep(home)
        .arg("--format")
        .arg("json")
        .arg("barks")
        .output()
        .unwrap();
    assert_success(&output);
    assert!(
        !home.join("run").join("shep.sock").exists(),
        "`shep barks` must never autostart a shepherd either"
    );

    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        envelope["command"], "barks",
        "`shep barks` must reach the barks verb and no other: {envelope}"
    );
    let rows = envelope["data"]
        .as_array()
        .unwrap_or_else(|| panic!("barks data must be an array: {envelope}"));
    assert_eq!(rows.len(), 1, "{envelope}");
    assert_eq!(rows[0]["subject"], "web", "{envelope}");
    assert_eq!(rows[0]["rule"], "watchdog", "{envelope}");
}
