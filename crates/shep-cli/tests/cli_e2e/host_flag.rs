//! `--host`: `flock` lists the host's flock through a stand-in `ssh`, and
//! every other verb refuses before it acts on this machine.
//!
//! Unix only, like [`super::ssh_stand_in`], whose script this reuses.

use super::*;

#[cfg(unix)]
use super::ssh_stand_in::write_ssh_stand_in;

#[cfg(unix)]
/// `PATH` with `dir` first, so the child finds the stand-in as `ssh`.
fn path_with_first(dir: &Path) -> std::ffi::OsString {
    let rest = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(std::iter::once(dir.to_owned()).chain(std::env::split_paths(&rest)))
        .unwrap()
}

#[cfg(unix)]
/// Starts a sheep named `name` on `home`, adopting the home into `guard`.
fn start_sheep(guard: &mut DaemonGuard, home: &TempDir, name: &str) {
    let script = write_script(
        home,
        &format!("{name}.sh"),
        &format!("{}{}", script_header(), sleep_line(SCRIPT_SLEEP_SECS)),
    );
    let started = shep(home.path())
        .arg("start")
        .arg(&script)
        .output()
        .unwrap();
    guard.adopt_home(home.path());
    assert_success(&started);
}

#[cfg(unix)]
/// The sheep names in a `--format json` flock envelope, dogs left out: every
/// shepherd lists its built-in dogs beside the flock.
fn json_names(output: &Output) -> Vec<String> {
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout was not JSON: {e}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    envelope["data"]
        .as_array()
        .unwrap_or_else(|| panic!("no data array: {envelope}"))
        .iter()
        .filter(|info| info["dog"].is_null())
        .map(|info| info["name"].as_str().unwrap().to_owned())
        .collect()
}

/// Each home runs a different sheep, so the listing says which shepherd
/// answered. Both formats, because the JSON path has its own failure shape.
#[cfg(unix)]
#[test]
fn flock_over_a_stand_in_ssh_lists_the_remote_flock_not_the_local_one() {
    let local = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let mut guard = DaemonGuard::default();
    start_sheep(&mut guard, &local, "local-sheep");
    start_sheep(&mut guard, &remote, "remote-sheep");
    write_ssh_stand_in(bin.path(), remote.path());
    let path = path_with_first(bin.path());

    let table = shep(local.path())
        .args(["--host", "amp2", "flock"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_success(&table);
    let text = String::from_utf8_lossy(&table.stdout);
    assert!(text.contains("remote-sheep"), "{text}");
    assert!(!text.contains("local-sheep"), "{text}");

    let json = shep(local.path())
        .args(["--format", "json", "--host", "amp2", "flock"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_success(&json);
    assert_eq!(json_names(&json), ["remote-sheep"]);

    graceful_kill(local.path());
    graceful_kill(remote.path());
}

/// The local home holds a roll. A dead host that printed it would list this
/// machine's flock under a host's name.
#[cfg(unix)]
#[test]
fn a_dead_remote_exits_5_and_prints_no_local_roll() {
    let local = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let roll =
        shep_daemon::snapshot::FlockSnapshot::with_apps(vec![shep_daemon::snapshot::SavedApp {
            app: shep_core::config::AppConfig::minimal("local-sheep", "./srv"),
            instances_running: 1,
            disabled: false,
        }]);
    std::fs::write(
        local.path().join("flock.json"),
        serde_json::to_vec(&roll).unwrap(),
    )
    .unwrap();
    let ssh = bin.path().join("ssh");
    std::fs::write(
        &ssh,
        "#!/bin/sh\necho 'ssh: connect to host amp2 port 22: Connection refused' >&2\nexit 255\n",
    )
    .unwrap();
    std::fs::set_permissions(&ssh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = path_with_first(bin.path());

    let table = shep(local.path())
        .args(["--host", "amp2", "flock"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_eq!(table.status.code(), Some(5));
    assert!(
        table.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&table.stdout)
    );
    let err = String::from_utf8_lossy(&table.stderr);
    assert!(err.contains("amp2"), "{err}");
    assert!(
        !err.contains("local-sheep") && !err.contains("saved roll"),
        "{err}"
    );

    let json = shep(local.path())
        .args(["--format", "json", "--host", "amp2", "flock"])
        .env("PATH", &path)
        .output()
        .unwrap();
    assert_json_error(&json, 5, "daemon_unreachable");
}

/// The dangerous failure: a verb that ignores `--host` and acts here. The
/// local home has no sheep by that name, so acting here would exit 3, and
/// the remote sheep would still be up either way. Exit 2 and a remote flock
/// that still lists it online pin both halves.
#[cfg(unix)]
#[test]
fn another_verb_with_host_exits_2_and_the_remote_sheep_is_untouched() {
    let local = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    let bin = tempfile::tempdir().unwrap();
    let mut guard = DaemonGuard::default();
    start_sheep(&mut guard, &local, "local-sheep");
    start_sheep(&mut guard, &remote, "remote-sheep");
    write_ssh_stand_in(bin.path(), remote.path());

    let refused = shep(local.path())
        .args(["--host", "amp2", "stop", "remote-sheep"])
        .env("PATH", path_with_first(bin.path()))
        .output()
        .unwrap();
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stderr={}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let err = String::from_utf8_lossy(&refused.stderr);
    assert!(
        err.contains("`--host` is not supported by this command yet"),
        "{err}"
    );

    let remote_flock = shep(remote.path())
        .args(["--format", "json", "flock"])
        .output()
        .unwrap();
    assert_success(&remote_flock);
    let envelope: serde_json::Value = serde_json::from_slice(&remote_flock.stdout).unwrap();
    let sheep = envelope["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|info| info["name"] == "remote-sheep")
        .unwrap_or_else(|| panic!("the remote sheep is gone: {envelope}"));
    assert_eq!(sheep["status"], "online", "{envelope}");

    let local_flock = shep(local.path())
        .args(["--format", "json", "flock"])
        .output()
        .unwrap();
    assert_success(&local_flock);
    assert_eq!(json_names(&local_flock), ["local-sheep"]);

    graceful_kill(local.path());
    graceful_kill(remote.path());
}
