//! `shep serve`: the port it answers on, the docroot it refuses, the
//! symlinks it follows, and how it stops.

use super::*;

#[cfg(unix)]
/// The assertion is an HTTP GET against the port, not a `shep flock` row: a
/// row says the process is up, and up is not serving.
#[test]
fn serve_registers_a_sheep_that_answers_on_its_port() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("site");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("index.html"), "hello from shep serve").unwrap();
    let mut guard = DaemonGuard::default();
    let port = free_port();

    let output = shep(dir.path())
        .arg("--format")
        .arg("json")
        .arg("serve")
        .arg(&root)
        .arg("--port")
        .arg(port.to_string())
        .output()
        .unwrap();
    guard.adopt_home(dir.path());
    assert_success(&output);

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (status, body) = poll_http_get(addr, "/", &[]);
    assert_eq!(status, 200, "body={body}");
    assert!(body.contains("hello from shep serve"), "{body}");

    graceful_kill(dir.path());
}

#[cfg(unix)]
#[test]
fn serve_refuses_a_docroot_that_is_not_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let mut guard = DaemonGuard::default();

    let output = shep(dir.path())
        .arg("--format")
        .arg("json")
        .arg("serve")
        .arg(&missing)
        .output()
        .unwrap();
    guard.adopt_home(dir.path());

    assert_json_error(&output, 2, "usage");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&missing.display().to_string()), "{stderr}");

    // No daemon was ever spawned to register anything against, so the
    // refusal happened before any `Request::Start`.
    assert!(
        daemon_pid(dir.path()).is_none(),
        "a refused root must not even bring a shepherd up"
    );
}

#[cfg(unix)]
/// Which rung of the kill ladder answered, read off the shepherd's own exit
/// record rather than a wall clock.
///
/// `Command::Stop` defers its reply until the sheep has exited, and
/// [`shep_core::protocol::ProcessInfo::last_exit`] carries why it exited:
/// the rung is in the reply the verb already prints. A worker that handles
/// `SIGTERM` leaves on its own terms, so the record is a code and no signal.
/// One that only handles `SIGINT` rides the ladder to `kill_tree`'s `SIGKILL`
/// and records signal 9 with no code. `docs/testing.md` puts a duration a
/// contended runner cannot hold still in the slow tier; the rung is the same
/// fact with no clock in it.
///
/// The `code` half is not decoration. A worker that never bound its listener
/// exits 1, and a case that only asked "signal, or no signal" would read that
/// crash as a handled `SIGTERM`.
#[test]
fn a_served_sheep_stops_on_sigterm_rather_than_riding_the_ladder_to_sigkill() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("site");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("index.html"), "ok").unwrap();
    let mut guard = DaemonGuard::default();
    let port = free_port();
    let name = "sigterm-check";

    let output = shep(dir.path())
        .arg("serve")
        .arg(&root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--name")
        .arg(name)
        .output()
        .unwrap();
    guard.adopt_home(dir.path());
    assert_success(&output);

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (status, body) = poll_http_get(addr, "/", &[]);
    assert_eq!(status, 200, "body={body}");

    let stop_output = shep(dir.path())
        .arg("--format")
        .arg("json")
        .arg("stop")
        .arg(name)
        .output()
        .unwrap();
    assert_success(&stop_output);
    let envelope: serde_json::Value = serde_json::from_slice(&stop_output.stdout)
        .unwrap_or_else(|e| panic!("stop stdout was not JSON: {e}"));
    let row = sheep_named(&envelope["data"], name);

    assert_eq!(row["status"], "stopped", "{row}");
    assert_eq!(
        row["last_exit"],
        serde_json::json!({ "code": 0, "signal": null }),
        "a served sheep must stop on the kill ladder's first rung. \
         A worker that only handles SIGINT rides it to SIGKILL and records \
         `{{\"code\": null, \"signal\": 9}}` instead: {row}"
    );

    graceful_kill(dir.path());
}

#[cfg(unix)]
/// Layout shared by the two `--follow-symlinks` cases below: a dated release
/// directory holding `index.html`, and a `current` symlink pointing at it.
fn write_deploy_layout(root: &Path) {
    let release = root.join("releases").join("2026-08-15");
    std::fs::create_dir_all(&release).unwrap();
    std::fs::write(release.join("index.html"), "the deploy layout").unwrap();
    std::os::unix::fs::symlink(&release, root.join("current")).unwrap();
}

#[cfg(unix)]
/// Registered without `--follow-symlinks`. A registered sheep is a real child
/// with its own captured stderr, which is what `shep bleats` reads.
#[test]
fn a_refused_symlink_writes_the_path_and_the_flag_to_the_sheeps_bleats() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("site");
    std::fs::create_dir(&root).unwrap();
    write_deploy_layout(&root);
    let canonical_root = root.canonicalize().unwrap();
    let mut guard = DaemonGuard::default();
    let port = free_port();
    let name = "symlink-refused";

    let output = shep(dir.path())
        .arg("serve")
        .arg(&root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--name")
        .arg(name)
        .output()
        .unwrap();
    guard.adopt_home(dir.path());
    assert_success(&output);

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (status, body) = poll_http_get(addr, "/current/index.html", &[]);
    assert_eq!(status, 404, "body={body}");

    let bleats_output = bleats_no_follow_until_written(dir.path(), &[name, "--err"]);
    let bleats = String::from_utf8_lossy(&bleats_output.stdout);
    assert!(
        bleats.contains(&canonical_root.join("current").display().to_string()),
        "{bleats}"
    );
    assert!(bleats.contains("--follow-symlinks"), "{bleats}");

    graceful_kill(dir.path());
}

#[cfg(unix)]
/// One scenario: the flag that makes the deploy layout work is the flag
/// `follow_symlinks_notice` announces.
#[test]
fn a_served_sheep_with_follow_symlinks_serves_the_deploy_layout_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("site");
    std::fs::create_dir(&root).unwrap();
    write_deploy_layout(&root);
    let mut guard = DaemonGuard::default();
    let port = free_port();
    let name = "symlink-followed";

    let output = shep(dir.path())
        .arg("serve")
        .arg(&root)
        .arg("--port")
        .arg(port.to_string())
        .arg("--name")
        .arg(name)
        .arg("--follow-symlinks")
        .output()
        .unwrap();
    guard.adopt_home(dir.path());
    assert_success(&output);

    let addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let (status, body) = poll_http_get(addr, "/current/index.html", &[]);
    assert_eq!(status, 200, "body={body}");
    assert!(body.contains("the deploy layout"), "{body}");

    let bleats_output = bleats_no_follow_until_written(dir.path(), &[name, "--err"]);
    let bleats = String::from_utf8_lossy(&bleats_output.stdout);
    assert!(bleats.contains("--follow-symlinks"), "{bleats}");
    assert!(
        bleats.contains("race") || bleats.contains("TOCTOU"),
        "{bleats}"
    );

    graceful_kill(dir.path());
}
