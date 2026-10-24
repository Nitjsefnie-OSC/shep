//! `take_inherited_socket` against a real child, the way a dog finds the
//! connection its shepherd spawned it holding.
//!
//! The test re-execs its own binary, as `real_child.rs` does. The child
//! reports what it found on stdout, one word per line.

#![cfg(all(unix, feature = "client"))]

use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Output, Stdio};

use command_fds::{CommandFdExt as _, FdMapping};
use shep_channel::{ChannelError, take_inherited_socket};

const VAR: &str = "SHEP_TEST_LINK_FD";
const CHILD_VAR: &str = "SHEP_INHERITED_SOCKET_CHILD";

/// The child's descriptor for the socket, above the standard streams and
/// the shepherd channel's own 3.
const LINK_FD: i32 = 4;

/// One line for the parent. Starts on a fresh line: libtest has already
/// written `test <name> ... ` without a newline.
fn report(line: &str) {
    println!("\nreported {line}");
}

/// Runs in the re-exec'd binary and never returns.
fn run_as_child() -> ! {
    let mut stream = match take_inherited_socket(VAR) {
        Ok(Some(stream)) => stream,
        Ok(None) => {
            report("none");
            std::process::exit(0);
        }
        Err(error) => {
            report(&format!("refused: {error}"));
            std::process::exit(0);
        }
    };
    let second = match take_inherited_socket(VAR) {
        Err(ChannelError::AlreadyTaken) => "second: already taken",
        _ => "second: not refused",
    };
    stream.write_all(b"hello shepherd").expect("write");
    report(second);
    std::process::exit(0);
}

fn spawn_child(test: &str, var: Option<&str>, fd: Option<OwnedFd>) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("test binary"));
    command
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(CHILD_VAR, "1")
        .env_remove(VAR)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(var) = var {
        command.env(VAR, var);
    }
    if let Some(parent_fd) = fd {
        command
            .fd_mappings(vec![FdMapping {
                parent_fd,
                child_fd: LINK_FD,
            }])
            .expect("map the socket");
    }
    command.output().expect("run the child")
}

/// What the child reported, in order.
fn reported(output: &Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("reported "))
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_child_takes_the_socket_the_variable_names_once() {
    if std::env::var_os(CHILD_VAR).is_some() {
        run_as_child();
    }
    let (mut ours, theirs) = UnixStream::pair().expect("socketpair");

    let output = spawn_child(
        "a_child_takes_the_socket_the_variable_names_once",
        Some("4"),
        Some(OwnedFd::from(theirs)),
    );

    assert_eq!(reported(&output), ["second: already taken"]);
    let mut heard = String::new();
    ours.read_to_string(&mut heard).expect("read what it wrote");
    assert_eq!(heard, "hello shepherd");
}

#[test]
fn a_child_without_the_variable_has_no_socket() {
    if std::env::var_os(CHILD_VAR).is_some() {
        run_as_child();
    }

    let output = spawn_child("a_child_without_the_variable_has_no_socket", None, None);

    assert_eq!(reported(&output), ["none"]);
}

#[test]
fn a_child_refuses_a_variable_that_is_not_a_descriptor_number() {
    if std::env::var_os(CHILD_VAR).is_some() {
        run_as_child();
    }

    let output = spawn_child(
        "a_child_refuses_a_variable_that_is_not_a_descriptor_number",
        Some("fido"),
        None,
    );

    assert_eq!(
        reported(&output),
        ["refused: unusable shepherd channel: SHEP_TEST_LINK_FD=fido"]
    );
}

#[test]
fn a_child_refuses_a_descriptor_that_is_not_a_socket() {
    if std::env::var_os(CHILD_VAR).is_some() {
        run_as_child();
    }
    let (read, _write) = std::io::pipe().expect("pipe");

    let output = spawn_child(
        "a_child_refuses_a_descriptor_that_is_not_a_socket",
        Some("4"),
        Some(OwnedFd::from(read)),
    );

    let lines = reported(&output);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("refused: unusable shepherd channel:"),
        "{lines:?}"
    );
}
