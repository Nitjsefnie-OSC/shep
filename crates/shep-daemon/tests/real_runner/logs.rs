//! The log plane over a real child: what its stdout and stderr
//! become, and where a relative log path lands.

use super::*;

#[tokio::test]
async fn exit_code_and_logs_are_captured() {
    let dir = tempfile::tempdir().unwrap();
    let out_file = dir.path().join("out.log");
    let err_file = dir.path().join("err.log");
    let runner = TokioRunner::new();
    let spec = sh_spec(
        "echo out-line; echo err-line 1>&2; exit 7",
        false,
        out_file.clone(),
        err_file.clone(),
    );

    let (mut proc, mut io) = runner.spawn(&spec).unwrap();

    let mut saw_out = false;
    let mut saw_err = false;
    for _ in 0..2 {
        let line = io
            .logs
            .recv()
            .await
            .expect("logs channel closed before both lines arrived");
        match (line.err, line.line.as_str()) {
            (false, "out-line") => saw_out = true,
            (true, "err-line") => saw_err = true,
            other => panic!("unexpected log line: {other:?}"),
        }
    }
    assert!(saw_out, "missing stdout line");
    assert!(saw_err, "missing stderr line");

    let outcome = proc.wait().await;
    assert_eq!(outcome.code, Some(7));
    assert_eq!(outcome.signal, None);

    // A line on `logs` means the pump issued the write, not that it landed:
    // `write_all().await` returning only means queued in `tokio::fs`'s
    // buffer. Read the file back on a bounded poll instead of assuming it.
    await_file_contents(&out_file, "out-line\n").await;
    await_file_contents(&err_file, "err-line\n").await;
}

/// A shell fragment that blocks until `marker` exists, polling once a
/// second: `sleep`'s only portable argument is a whole number of seconds.
///
/// Lets the test control exactly when a child prints its next line,
/// without guessing at timing.
fn wait_for_marker(marker: &Path) -> String {
    format!("while [ ! -f {} ]; do sleep 1; done", marker.display())
}

/// Fails if a reopen leaves the pump writing into the renamed inode,
/// silently producing an empty live log forever under `create`-mode
/// rotation.
///
/// Checks both halves: the archive must stop growing and the live path
/// must get the new lines. A pump holding a second handle to the old file
/// would still pass a check of only one side.
#[tokio::test]
async fn a_reopen_moves_a_real_childs_output_onto_the_recreated_path() {
    let dir = tempfile::tempdir().unwrap();
    let out_file = dir.path().join("out.log");
    let err_file = dir.path().join("err.log");
    let marker = dir.path().join("go");
    let runner = TokioRunner::new();
    let spec = sh_spec(
        &format!("echo before; {}; echo after", wait_for_marker(&marker)),
        false,
        out_file.clone(),
        err_file.clone(),
    );

    let (mut proc, mut io) = runner.spawn(&spec).unwrap();
    // The child blocks on the marker, so any assertion that fails before
    // it is written leaves a real process behind for the rest of the run.
    let _reaper = Reaper::of(&proc);

    let line = recv_log(
        &mut io,
        LOG_WRITE_DEADLINE,
        "the child's first line must arrive",
    )
    .await;
    assert_eq!(line.line, "before");
    await_file_contents(&out_file, "before\n").await;

    // The rotator's half: the pump's handle now names an inode the live
    // path no longer resolves to.
    let archive = dir.path().join("out.log.1");
    fs::rename(&out_file, &archive).unwrap();
    assert!(!out_file.exists(), "sanity: the rename really moved it");

    let (done, ack) = tokio::sync::oneshot::channel();
    io.log_ctl
        .send(shep_daemon::runner::LogCtl::Reopen { done })
        .await
        .expect("a running sheep's pump must still be reachable");
    let outcome = tokio::time::timeout(LOG_WRITE_DEADLINE, ack)
        .await
        .expect("the reopen must be acknowledged")
        .expect("the pump must answer rather than drop the acknowledgement");
    assert_eq!(
        outcome,
        Ok(()),
        "the live path is there to be opened: the rename moved the inode, not the directory"
    );

    // No polling: the acknowledgement is a real barrier, since the reopen
    // flushes the old handle before dropping it.
    assert_eq!(unstamped_file(&out_file), "");
    assert_eq!(unstamped_file(&archive), "before\n");

    fs::write(&marker, "").unwrap();
    let line = recv_log(
        &mut io,
        LOG_WRITE_DEADLINE,
        "the child's second line must arrive",
    )
    .await;
    assert_eq!(line.line, "after");

    await_file_contents(&out_file, "after\n").await;
    assert_eq!(
        unstamped_file(&archive),
        "before\n",
        "the renamed file must stop growing the moment the handle is swapped"
    );

    let outcome = reaped(
        &mut proc,
        REAP_DEADLINE,
        "the child exits once it has printed its second line",
    )
    .await;
    assert_eq!(outcome.code, Some(0));
}

/// The shepherd opens a sheep's log files itself, so a relative `out_file`
/// has to be anchored at the sheep's `cwd` before that open.
///
/// This test process stands somewhere else entirely, which is the case: an
/// unanchored path would create the file under the test binary's own
/// directory and leave the one asserted here missing.
#[tokio::test]
async fn a_relative_log_path_lands_under_the_sheep_cwd() {
    use shep_core::config::{AppConfig, normalize};
    use shep_core::paths::ShepPaths;
    use shep_core::secrets::SecretView;
    use shep_daemon::assemble::assemble;

    let dir = tempfile::tempdir().unwrap();
    let app_dir = dir.path().join("app");
    fs::create_dir_all(&app_dir).unwrap();
    let paths = ShepPaths::resolve(
        &|key| (key == "SHEP_HOME").then(|| dir.path().to_string_lossy().into_owned()),
        Path::new("/unused-by-this-fixture"),
    );

    let app_config = AppConfig {
        name: "anchored".to_string(),
        script: "/bin/sh".to_string(),
        interpreter: Some("none".to_string()),
        args: vec!["-c".to_string(), "echo anchored-ok".to_string()],
        cwd: Some(app_dir.to_string_lossy().into_owned()),
        out_file: Some("logs/out.log".to_string()),
        err_file: Some("logs/err.log".to_string()),
        ..Default::default()
    };
    let app = normalize(app_config).unwrap();
    let spec = assemble(
        &app,
        0,
        &paths,
        None,
        &SecretView::empty("production".to_string()),
    )
    .expect("this fixture carries no secret to resolve");
    assert_eq!(spec.out_file, app_dir.join("logs").join("out.log"));

    let runner = TokioRunner::new();
    let (mut proc, _io) = runner.spawn(&spec).unwrap();
    assert_eq!(proc.wait().await.code, Some(0));
    await_file_contents(&app_dir.join("logs").join("out.log"), "anchored-ok\n").await;
}
