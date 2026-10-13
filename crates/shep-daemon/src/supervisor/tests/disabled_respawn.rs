//! The respawn path against a disabled app, with the actor driven directly.
//!
//! The doors in front of it (`restart_unless_disabled`, the exit branch)
//! already answer for a disabled app, so these cases mark the app without a
//! stop in flight, a state only a missed door could reach. Each fails if the
//! guard in `respawn`, or the extra restart's own, goes.

use super::*;

/// No scripts: a spawn the guard failed to stop lands `Errored`, not
/// `Stopped`.
fn disabled_waiting_restart(
    dir: &tempfile::TempDir,
    epoch: u64,
) -> (Actor<ScriptedRunner>, mpsc::Receiver<Msg>) {
    let (mut actor, rx) = actor_with_one_online_sheep(dir, Vec::new());
    let slot = actor.sheep.get_mut(&0).expect("the fixture registers id 0");
    slot.entry.status = ProcStatus::WaitingRestart;
    slot.entry.pid = None;
    slot.epoch = epoch;
    actor.disabled.insert("web".to_string());
    (actor, rx)
}

// The crash ladder's backoff timer for an app disabled in the meantime.
#[test]
fn a_backoff_timer_leaves_a_disabled_app_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _rx) = disabled_waiting_restart(&dir, 7);

    actor.handle_restart_due(0, 7);

    let slot = actor.sheep.get(&0).expect("the sheep stays registered");
    assert_eq!(slot.entry.status, ProcStatus::Stopped);
    assert_eq!(slot.entry.restarts, 0, "a dropped timer must never respawn");
    assert_eq!(slot.entry.pid, None);
}

// A cron occurrence or a watch trigger that lands on a stopped, disabled app
// restarts it inline through `apply_immediate`.
#[test]
fn an_automatic_restart_leaves_a_stopped_disabled_app_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _rx) = actor_with_one_online_sheep(&dir, Vec::new());
    let slot = actor.sheep.get_mut(&0).expect("the fixture registers id 0");
    slot.entry.status = ProcStatus::Stopped;
    slot.entry.pid = None;
    actor.disabled.insert("web".to_string());

    let info = actor
        .apply_immediate(0, ManualKind::Restart, CommandOrigin::Automatic)
        .expect("the sheep is registered");

    assert_eq!(info.status, ProcStatus::Stopped);
    assert_eq!(info.restarts, 0, "a dropped restart must never respawn");
}

// A memory breach or a liveness failure reported against a disabled app
// whose process is still up.
#[test]
fn an_extra_restart_leaves_a_disabled_app_alone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut actor, _rx) = actor_with_one_online_sheep(&dir, Vec::new());
    let (ctl_tx, mut ctl_rx) = mpsc::channel(1);
    actor
        .sheep
        .get_mut(&0)
        .expect("the fixture registers id 0")
        .ctl = Some(ctl_tx);
    actor.disabled.insert("web".to_string());

    actor.handle_extra_restart(0, 1111, None, None);

    let slot = actor.sheep.get(&0).expect("the sheep stays registered");
    assert!(
        slot.manual.is_none(),
        "a disabled app must never claim the manual marker off an extra restart"
    );
    assert!(
        ctl_rx.try_recv().is_err(),
        "a disabled app must never receive a Kill from an extra restart"
    );
}
