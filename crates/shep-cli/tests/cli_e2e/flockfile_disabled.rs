//! Re-running a Flockfile that holds a sheep `shep disable` parked.

use super::*;

/// The re-run starts the file's new app, leaves the disabled sheep down,
/// says how to bring it back, and exits 0.
#[test]
fn a_rerun_passes_a_disabled_sheep_by_and_starts_the_new_app() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let app = |name: &str| {
        format!(
            "[[app]]\nname = \"{name}\"\nscript = '{}'\n\n",
            script.display()
        )
    };
    let flockfile = write_flockfile(&dir, &app("parked"));
    let mut guard = DaemonGuard::default();

    let boot = shep(home)
        .arg("start")
        .arg(&flockfile)
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    guard.adopt_home(home);
    assert_success(&boot);
    poll_flock(home, |info| info["status"] == "online");

    let disable = shep(home)
        .args(["disable", "parked"])
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    assert_success(&disable);

    write_flockfile(&dir, &format!("{}{}", app("parked"), app("fresh")));
    let again = shep(home)
        .arg("start")
        .arg(&flockfile)
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();

    assert_success(&again);
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert!(
        stderr.contains("`shep enable parked`"),
        "the aside names the way back: {stderr}"
    );
    let data = poll_flock_data(home, FLOCK_DEADLINE, |data| {
        sheep_named(data, "fresh")["status"] == "online"
    });
    let parked = sheep_named(&data, "parked");
    assert_eq!(parked["status"], "stopped", "{data}");
    assert_eq!(parked["disabled"], true, "{data}");
}
