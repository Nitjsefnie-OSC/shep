//! A sheep `shep disable` parked, through `shep daemon reload`.

use super::*;

/// `parked` is still stopped, still marked and still refused by `restart`
/// after the successor took the flock over.
///
/// The other sheep keeping its pid is what makes this a handover rather than a
/// stop-and-start: the stop arm would have re-read the mark off the roll, which
/// is #765's path, and proved nothing about the blob.
///
/// Unix only, as the whole handover is: Windows has no `execve`.
#[cfg(unix)]
#[test]
fn a_disabled_sheep_stays_disabled_across_a_daemon_reload() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let app = |name: &str| {
        format!(
            "[[app]]\nname = \"{name}\"\nscript = '{}'\n\n",
            script.display()
        )
    };
    let flockfile = write_flockfile(&dir, &format!("{}{}", app("parked"), app("kept")));
    let mut guard = DaemonGuard::default();

    let boot = shep(home)
        .arg("start")
        .arg(&flockfile)
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    guard.adopt_home(home);
    assert_success(&boot);

    let before = poll_flock_data(home, FLOCK_DEADLINE, |data| {
        data.as_array().is_some_and(|rows| {
            rows.len() == 2
                && rows
                    .iter()
                    .all(|row| row["status"] == "online" && !row["pid"].is_null())
        })
    });
    let pid_kept = sheep_named(&before, "kept")["pid"]
        .as_u64()
        .unwrap_or_else(|| panic!("an online sheep reports a pid: {before}"));

    let disabled = shep(home)
        .args(["disable", "parked"])
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    assert_success(&disabled);
    let parked_before = poll_flock_data(home, FLOCK_DEADLINE, |data| {
        sheep_named(data, "parked")["status"] == "stopped"
    });
    let row = sheep_named(&parked_before, "parked");
    assert_eq!(row["disabled"].as_bool(), Some(true), "{parked_before}");

    let reloaded = shep(home)
        .arg("daemon")
        .arg("reload")
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    assert_success(&reloaded);

    let after = poll_flock_data(home, FLOCK_DEADLINE, |data| {
        data.as_array().is_some_and(|rows| {
            rows.len() == 2
                && sheep_named(data, "kept")["status"] == "online"
                && sheep_named(data, "parked")["status"] == "stopped"
        })
    });
    assert_eq!(
        sheep_named(&after, "kept")["pid"].as_u64(),
        Some(pid_kept),
        "`kept` was respawned, so this reload took the stop arm and never \
         handed the mark over: {after}"
    );
    let parked = sheep_named(&after, "parked");
    assert_eq!(parked["status"], "stopped", "{after}");
    assert_eq!(parked["disabled"].as_bool(), Some(true), "{after}");
    assert!(parked["pid"].is_null(), "{after}");

    // The successor's guard, not just its listing row: a mark that only
    // reached the reply would look identical here.
    let refused = shep(home)
        .arg("restart")
        .arg("parked")
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2), "{refused:?}");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("`shep enable parked`"),
        "the refusal names the way back: {stderr}"
    );
    let still = poll_flock_data(home, FLOCK_DEADLINE, |data| {
        sheep_named(data, "parked")["status"] == "stopped"
    });
    let parked = sheep_named(&still, "parked");
    assert_eq!(parked["status"], "stopped", "{still}");
    assert_eq!(parked["disabled"].as_bool(), Some(true), "{still}");

    graceful_kill(home);
}
