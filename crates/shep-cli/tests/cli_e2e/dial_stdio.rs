//! `shep dial-stdio`: the bytes it carries to a shepherd and how it ends.
//!
//! Unix only: the Windows named-pipe leg is a human check once the ssh
//! transport lands.

use super::*;

#[cfg(unix)]
use std::io::Write as _;
#[cfg(unix)]
use std::process::{ChildStdin, ChildStdout};

#[cfg(unix)]
use shep_core::protocol::{
    HelloReply, PROTOCOL_VERSION, decode_frame, encode_frame, request::Hello,
};

/// Bound on one frame arriving through the bridge, and on the bridge ending.
#[cfg(unix)]
const BRIDGE_DEADLINE: Duration = Duration::from_secs(10);

/// No shepherd is a failed probe, and it must leave nothing behind: stdout is
/// the frame channel, and the verb is run on machines that may have no shep
/// home at all.
#[cfg(unix)]
#[test]
fn dial_stdio_with_no_shepherd_exits_5_leaves_stdout_empty_and_creates_no_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("no-such-home");

    let output = shep(&home).arg("dial-stdio").output().unwrap();

    assert_eq!(output.status.code(), Some(5), "{output:?}");
    assert!(output.stdout.is_empty(), "stdout: {:?}", output.stdout);
    assert!(
        !String::from_utf8_lossy(&output.stderr).is_empty(),
        "the failure must be explained on stderr"
    );
    assert!(!home.exists(), "dial-stdio created the home");
}

#[cfg(unix)]
/// Starts a shepherd on `home` and spawns `shep dial-stdio` against it with
/// all three stdio streams piped.
fn dial_a_running_shepherd(home: &TempDir, guard: &mut DaemonGuard) -> Child {
    let script = write_test_script(home);
    let started = shep(home.path())
        .arg("start")
        .arg(&script)
        .output()
        .unwrap();
    guard.adopt_home(home.path());
    assert_success(&started);

    std::process::Command::cargo_bin("shep")
        .unwrap()
        .arg("--home")
        .arg(home.path())
        .arg("dial-stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

#[cfg(unix)]
/// Sends a framed `Hello` down `stdin` and returns the shepherd's answer
/// along with `stdout`, which carries the rest of the conversation.
///
/// The read runs on its own thread so a bridge that carries nothing fails as
/// a named timeout rather than a blocked test.
fn say_hello(stdin: &mut ChildStdin, mut stdout: ChildStdout) -> (HelloReply, ChildStdout) {
    let hello = Hello {
        client_version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol: PROTOCOL_VERSION,
        dog_name: None,
    };
    let payload = encode_frame(&hello).unwrap();
    stdin
        .write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes())
        .unwrap();
    stdin.write_all(&payload).unwrap();
    stdin.flush().unwrap();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut prefix = [0_u8; 4];
        let mut frame = Vec::new();
        let read = stdout.read_exact(&mut prefix).and_then(|()| {
            frame.resize(u32::from_be_bytes(prefix) as usize, 0);
            stdout.read_exact(&mut frame)
        });
        let _ = tx.send(read.map(|()| (frame, stdout)));
    });
    let (frame, stdout) = rx
        .recv_timeout(BRIDGE_DEADLINE)
        .expect("no frame came back through dial-stdio in time")
        .expect("read the reply frame");
    (decode_frame(&frame).unwrap(), stdout)
}

/// The frames cross unchanged: a real `Hello` in, a real `HelloAck` out, and
/// closing stdin is an orderly end.
#[cfg(unix)]
#[test]
fn dial_stdio_carries_a_real_handshake_to_a_running_shepherd() {
    let home = tempfile::tempdir().unwrap();
    let mut guard = DaemonGuard::default();
    let mut child = dial_a_running_shepherd(&home, &mut guard);
    let mut stdin = child.stdin.take().unwrap();

    let (reply, _stdout) = say_hello(&mut stdin, child.stdout.take().unwrap());

    let ack = reply.expect("the shepherd accepted this build's Hello");
    assert_eq!(ack.daemon_version, env!("CARGO_PKG_VERSION"));
    drop(stdin);
    let status = common::wait_bounded(&mut child, BRIDGE_DEADLINE, "shep dial-stdio");
    assert!(
        status.success(),
        "closing stdin is an orderly end: {status:?}"
    );
    graceful_kill(home.path());
}

/// The process must end when the shepherd goes away, though stdin stays open:
/// ssh keeps its end of the pipe open for as long as the local client lives.
///
/// The handshake comes first, so the bridge is known to be connected: a
/// `dial-stdio` that never reached the shepherd would also exit promptly.
#[cfg(unix)]
#[test]
fn dial_stdio_exits_promptly_when_the_shepherd_is_killed_with_stdin_still_open() {
    let home = tempfile::tempdir().unwrap();
    let mut guard = DaemonGuard::default();
    let mut child = dial_a_running_shepherd(&home, &mut guard);
    let mut stdin = child.stdin.take().unwrap();
    let (reply, _stdout) = say_hello(&mut stdin, child.stdout.take().unwrap());
    reply.expect("the shepherd accepted this build's Hello");

    let daemon = daemon_pid(home.path()).expect("a running shepherd records its pid");
    nix::sys::signal::kill(daemon, nix::sys::signal::Signal::SIGKILL).unwrap();

    common::wait_bounded(&mut child, BRIDGE_DEADLINE, "shep dial-stdio");
    drop(stdin);
}
