//! The built-in dogs ask like any other and run on exactly what they
//! declare: the first `shep enable` prompts, `--yes` grants the declared
//! list, and the real dog, held to that list by the real shepherd, still does
//! its job.

use super::*;

/// What `metrics` declares, as `shep.toml` records it.
const METRICS_GRANTS: &str =
    r#"granted = ["events:channel.metric", "events:process.", "flock.read"]"#;

/// What `bark` declares, as `shep.toml` records it.
const BARK_GRANTS: &str = r#"granted = ["events:log.", "events:process.", "flock.read"]"#;

fn shep_toml(home: &Path) -> String {
    std::fs::read_to_string(home.join("shep.toml")).unwrap_or_default()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A dog's own err log, empty until it has written to it.
fn dog_err_log(home: &Path, dog: &str) -> String {
    std::fs::read_to_string(home.join("logs").join(format!("{dog}-0-err.log"))).unwrap_or_default()
}

/// Waits for `dog`'s err log to hold `needle`.
fn wait_for_err_log(home: &Path, dog: &str, needle: &str) -> String {
    let deadline = Instant::now() + FLOCK_DEADLINE;
    loop {
        let log = dog_err_log(home, dog);
        if log.contains(needle) || Instant::now() >= deadline {
            return log;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
#[test]
fn the_first_enable_of_each_built_in_prompts_and_yes_grants_what_it_declares() {
    for (dog, grants) in [("metrics", METRICS_GRANTS), ("bark", BARK_GRANTS)] {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();

        let refused = shep(home).arg("enable").arg(dog).output().unwrap();
        assert!(!refused.status.success(), "{dog}: {}", stderr(&refused));
        assert!(stderr(&refused).contains("--yes"), "{}", stderr(&refused));
        assert!(!shep_toml(home).contains("dog_grants"), "{dog}");
        assert!(!shep_toml(home).contains(dog), "{dog}");

        let granted = shep(home)
            .arg("enable")
            .arg("--yes")
            .arg(dog)
            .output()
            .unwrap();
        assert_success(&granted);
        let listing = stderr(&granted);
        assert!(
            listing.contains("flock.read") && listing.contains("do not confine"),
            "{dog}: {listing}"
        );
        let written = shep_toml(home);
        assert!(
            written.contains(&format!("[daemon.dog_grants.{dog}]\n{grants}")),
            "{dog}: {written}"
        );
        assert!(written.contains(&format!("\"{dog}\"")), "{dog}: {written}");
    }
}

/// The scrape needs `ListFlock`, and the feed needs both subscriptions: a
/// metrics dog refused any of them exits or serves no sheep.
#[cfg(unix)]
#[test]
fn metrics_does_its_job_on_only_the_grants_it_declares() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let port = free_port();
    std::fs::write(
        home.join("dogs.toml"),
        format!("[metrics]\nbind = \"127.0.0.1:{port}\"\n"),
    )
    .unwrap();
    let script = write_test_script(&dir);
    let mut guard = DaemonGuard::default();

    let started = shep(home)
        .arg("start")
        .arg(&script)
        .arg("--name")
        .arg("web")
        .output()
        .unwrap();
    guard.adopt_home(home);
    assert_success(&started);
    poll_flock(home, |info| info["status"] == "online");

    assert_success(
        &shep(home)
            .arg("enable")
            .arg("--yes")
            .arg("metrics")
            .output()
            .unwrap(),
    );
    assert!(shep_toml(home).contains(METRICS_GRANTS));
    guard.adopt_dog_pid(wait_for_dog_pid(home, "metrics"));

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let body = poll_metrics_containing(
        addr,
        r#"shep_sheep_status{sheep="web",id="2",fold="",status="online"} 1"#,
    );
    assert!(body.contains("HTTP/1.1 200"), "{body}");
    assert!(
        !dog_err_log(home, "metrics").contains("was refused"),
        "{}",
        dog_err_log(home, "metrics")
    );

    graceful_kill(home);
}

/// Boots a shepherd with a bark whose one rule reads a chatty sheep's log,
/// so it sends every request bark can: the log subscriptions, the flock
/// names, the poll. `shep.toml` is `shep_toml`, which says what bark was
/// granted.
#[cfg(unix)]
fn boot_bark_reading_a_log(dir: &TempDir, shep_toml: &str, guard: &mut DaemonGuard) {
    let home = dir.path();
    std::fs::write(
        home.join("dogs.toml"),
        "[bark]\npoll = \"1s\"\n\n[bark.sinks.ops]\nkind = \"json\"\n\
         url = \"http://127.0.0.1:9/hook\"\n\n[[bark.rules]]\non = \"log_match\"\n\
         pattern = \"ALERTME\"\nsinks = [\"ops\"]\n",
    )
    .unwrap();
    let chatty = write_script(
        dir,
        "chatty.sh",
        &format!(
            "{}while true; do echo ALERTME; sleep 1; done\n",
            script_header()
        ),
    );
    std::fs::write(home.join("shep.toml"), shep_toml).unwrap();
    let started = shep(home)
        .arg("start")
        .arg(&chatty)
        .arg("--name")
        .arg("web")
        .output()
        .unwrap();
    guard.adopt_home(home);
    assert_success(&started);
}

/// A log rule makes bark subscribe to the log topics, read the flock's names
/// once, and poll. The bark in the history is the proof the subscription was
/// let through, and the quiet err log the proof the rest was.
#[cfg(unix)]
#[test]
fn bark_does_its_job_on_only_the_grants_it_declares() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut guard = DaemonGuard::default();
    boot_bark_reading_a_log(&dir, "", &mut guard);

    assert_success(
        &shep(home)
            .arg("enable")
            .arg("--yes")
            .arg("bark")
            .output()
            .unwrap(),
    );
    assert!(shep_toml(home).contains(BARK_GRANTS));
    guard.adopt_dog_pid(wait_for_dog_pid(home, "bark"));

    let deadline = Instant::now() + FLOCK_DEADLINE;
    let barked = loop {
        let output = shep(home)
            .arg("--format")
            .arg("json")
            .arg("barks")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        if text.contains("log_match") || Instant::now() >= deadline {
            break text;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(barked.contains("log_match"), "{barked}");

    // Past a poll or two, so a refused `ListFlock` has been reported.
    std::thread::sleep(Duration::from_millis(2500));
    let log = dog_err_log(home, "bark");
    assert!(!log.contains("was refused"), "{log}");

    graceful_kill(home);
}

/// The control for the case above: the same bark under the same shepherd
/// without `events:log.` is refused it, so that case proves something.
#[cfg(unix)]
#[test]
fn bark_without_one_of_its_declared_grants_is_refused_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let mut guard = DaemonGuard::default();
    boot_bark_reading_a_log(
        &dir,
        "[daemon]\nenabled_dogs = [\"bark\"]\n\n[daemon.dog_grants.bark]\n\
         granted = [\"events:process.\", \"flock.read\"]\n",
        &mut guard,
    );
    guard.adopt_home(home);

    let log = wait_for_err_log(home, "bark", "was refused");
    assert!(log.contains("events:log."), "{log}");

    graceful_kill(home);
}
