//! Signals into a real process tree: one the shell ignores, one aimed
//! at the sheep and not its lamb, and a graceful stop that must reach
//! a forked grandchild.

use super::*;

#[tokio::test]
async fn signal_ignored_then_kill_tree_reaps() {
    let dir = tempfile::tempdir().unwrap();
    let runner = TokioRunner::new();
    let spec = sh_spec(
        r#"trap "" TERM; while true; do sleep 1; done"#,
        false,
        dir.path().join("out.log"),
        dir.path().join("err.log"),
    );

    let (mut proc, _io) = runner.spawn(&spec).unwrap();

    // Gives the shell time to run `trap "" TERM` before we signal it, or
    // the signal can win the race and kill it via the untrapped default.
    tokio::time::sleep(Duration::from_millis(100)).await;
    proc.signal(StopSignal::Term).unwrap();

    // Gives the child a window to have exited if the signal wrongly
    // killed it despite the trap.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let still_running = tokio::time::timeout(Duration::from_millis(1), proc.wait()).await;
    assert!(
        still_running.is_err(),
        "process should still be running after an ignored SIGTERM"
    );

    proc.kill_tree().unwrap();
    let outcome = reaped(&mut proc, REAP_DEADLINE, "kill_tree should reap promptly").await;
    assert_eq!(outcome.signal, Some(9));
}

/// A real child, a real `kill(2)`, and the one assertion the scripted tier
/// cannot make: the signal reached the sheep, not the lamb it forked.
///
/// The wrapper traps SIGUSR1 and prints one word; the lamb traps it and
/// prints another. A group delivery prints both. Bounded: a signal that
/// reached nobody must fail the read rather than hang it.
#[tokio::test]
async fn a_process_signal_reaches_the_sheep_and_not_its_lamb() {
    let dir = tempfile::tempdir().unwrap();
    // The lamb announces itself once its trap is armed, so the test waits
    // for that instead of racing the fork.
    let script = r#"
        trap 'echo sheep-got-it' USR1
        ( trap 'echo lamb-got-it' USR1; echo lamb-ready; while :; do sleep 0.1; done ) &
        while :; do sleep 0.1; done
    "#;
    let spec = sh_spec(
        script,
        false,
        dir.path().join("out.log"),
        dir.path().join("err.log"),
    );
    let runner = TokioRunner::new();
    let (mut proc, mut io) = runner.spawn(&spec).unwrap();
    let _reaper = Reaper::of(&proc);

    let ready = recv_log(
        &mut io,
        LONG_DEADLINE,
        "the lamb did not announce itself within 10s",
    )
    .await;
    assert_eq!(ready.line, "lamb-ready");

    proc.signal_process(OperatorSignal::Usr1).unwrap();

    let answer = recv_log(
        &mut io,
        LONG_DEADLINE,
        "nothing answered the signal within 10s",
    )
    .await;
    assert_eq!(answer.line, "sheep-got-it");

    // And nothing else follows it. A group delivery would put `lamb-got-it` on
    // the same stream; a bounded read that times out is the proof it did not.
    let extra = tokio::time::timeout(Duration::from_secs(2), io.logs.recv()).await;
    assert!(
        extra.is_err(),
        "the lamb answered too: {extra:?} — the signal reached the group"
    );

    proc.kill_tree().unwrap();
}

/// How long the forked grandchild in
/// [`a_graceful_stop_reaches_a_forked_grandchild`] sleeps: longer than
/// [`super::REAP_DEADLINE`], so a pass proves the signal reached it rather than
/// that it exited on its own.
const ORPHAN_SLEEP_SECS: u32 = 30;

/// The wrapper forks a long-lived child without `exec`, so a leader-only
/// signal can let the wrapper exit clean while the fork runs on,
/// reparented and untracked.
///
/// Only the grandchild's death tells the two behaviors apart: the wrapper
/// exits on `SIGTERM` either way.
#[tokio::test]
async fn a_graceful_stop_reaches_a_forked_grandchild() {
    let dir = tempfile::tempdir().unwrap();
    let runner = TokioRunner::new();
    let spec = sh_spec(
        &format!("sleep {ORPHAN_SLEEP_SECS} & echo $!; wait"),
        false,
        dir.path().join("out.log"),
        dir.path().join("err.log"),
    );

    let (mut proc, mut io) = runner.spawn(&spec).unwrap();
    let leader = i32::try_from(proc.pid()).unwrap();
    let mut reaper = Reaper(vec![leader]);

    // Pins the property `-pid` signals depend on: without `process_group(0)`
    // the pid would lead no group at all.
    assert_eq!(
        nix::unistd::getpgid(Some(nix::unistd::Pid::from_raw(leader)))
            .unwrap()
            .as_raw(),
        leader,
        "a spawned sheep must lead its own process group"
    );

    // The wrapper prints `$!` only after forking, so receiving this line
    // proves the grandchild already exists.
    let line = recv_log(
        &mut io,
        LOG_WRITE_DEADLINE,
        "the wrapper must report its forked child's pid",
    )
    .await;
    let grandchild: i32 = line.line.trim().parse().expect("`echo $!` prints a pid");
    reaper.0.push(grandchild);
    assert_ne!(grandchild, leader, "sanity: `&` really forked");

    proc.signal(StopSignal::Term).unwrap();
    let outcome = reaped(&mut proc, REAP_DEADLINE, "the wrapper must exit on SIGTERM").await;
    assert_eq!(
        outcome.signal,
        Some(StopSignal::Term.as_raw()),
        "the wrapper itself dies of the same signal either way"
    );

    assert_reaped(grandchild, "the wrapper's forked child").await;
}
