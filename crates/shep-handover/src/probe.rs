//! Asking the target whether it can read the state, before the exec.
//!
//! The exec cannot be taken back, so a target that cannot read the state has
//! to be found out beforehand. The outgoing side starts the target as a child,
//! with the format in [`PROBE_ENV`]; the target's [`boot`](crate::boot) sees
//! the variable, answers with its exit code and exits. The child is the
//! target's own `main`, run only as far as `boot`.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::state::{HANDOVER_ENV, State};

/// The variable that makes [`boot`](crate::boot) answer a probe and exit. Its
/// value is the format the outgoing side writes.
pub(crate) const PROBE_ENV: &str = "SHEP_PROCESS_HANDOVER_PROBE";

/// The exit code for "this build reads that format".
const READS: i32 = 94;
/// The exit code for "this build cannot read that format".
const REFUSES: i32 = 95;

/// How long [`ask`] waits for an answer unless the caller says otherwise.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often [`ask`] looks for the child's exit.
const POLL: Duration = Duration::from_millis(5);

/// What a target said about a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reply {
    Reads,
    Refuses,
}

/// The exit code to answer a probe with, if `value` is one: [`PROBE_ENV`]'s
/// value, as the incoming side found it.
///
/// A value that is not a format number is no probe, so a stray variable
/// cannot turn a process into one that exits.
pub(crate) fn answer<S: State>(value: Option<&OsStr>) -> Option<i32> {
    let format: u32 = value?.to_str()?.parse().ok()?;
    Some(if S::reads(format) { READS } else { REFUSES })
}

/// Start `target` with `argv` as a probe for `format` and wait for its answer.
///
/// The child has no standard streams. One that has not answered within
/// `timeout` is killed and reaped.
///
/// # Errors
///
/// The target could not be started, did not exit in time, was ended by a
/// signal, or exited with a code that is not an answer: it never reached
/// [`boot`](crate::boot), which has to be the first thing its `main` does.
pub(crate) fn ask(
    target: &Path,
    argv: &[OsString],
    format: u32,
    timeout: Duration,
) -> io::Result<Reply> {
    let mut command = Command::new(target);
    if let Some((arg0, rest)) = argv.split_first() {
        command.arg0(arg0).args(rest);
    }
    let mut child = command
        .env(PROBE_ENV, format.to_string())
        .env_remove(HANDOVER_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("no answer within {timeout:?}"),
                ));
            }
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        }
    };
    match status.code() {
        Some(READS) => Ok(Reply::Reads),
        Some(REFUSES) => Ok(Reply::Refuses),
        Some(code) => Err(io::Error::other(format!(
            "it exited {code} without answering, so it never reached `boot`"
        ))),
        None => Err(io::Error::other(format!(
            "it was ended by a signal ({status})"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Serialize, Deserialize)]
    struct Two {}

    impl State for Two {
        const FORMAT: u32 = 2;
    }

    fn shell(script: &str) -> Vec<OsString> {
        ["sh", "-c", script].map(OsString::from).to_vec()
    }

    fn sh() -> &'static Path {
        Path::new("/bin/sh")
    }

    #[test]
    fn a_process_with_no_probe_variable_answers_nothing() {
        assert_eq!(answer::<Two>(None), None);
    }

    #[test]
    fn a_value_that_is_not_a_format_is_no_probe() {
        for value in ["", "x", "-1", "2.0", "4294967296"] {
            assert_eq!(answer::<Two>(Some(OsStr::new(value))), None, "{value}");
        }
    }

    #[test]
    fn a_probe_for_a_readable_format_is_answered_yes_and_for_another_no() {
        assert_eq!(answer::<Two>(Some(OsStr::new("2"))), Some(READS));
        assert_eq!(answer::<Two>(Some(OsStr::new("3"))), Some(REFUSES));
    }

    #[test]
    fn the_target_is_asked_with_the_format_in_its_environment() {
        let script = format!(r#"[ "${PROBE_ENV}" = 7 ] && exit {READS}; exit {REFUSES}"#);
        let reply = ask(sh(), &shell(&script), 7, Duration::from_secs(10)).unwrap();
        assert_eq!(reply, Reply::Reads);
    }

    #[test]
    fn a_target_that_refuses_the_format_says_so() {
        let reply = ask(
            sh(),
            &shell(&format!("exit {REFUSES}")),
            7,
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(reply, Reply::Refuses);
    }

    #[test]
    fn a_target_that_exits_without_answering_is_an_error_naming_boot() {
        let err = ask(sh(), &shell("exit 0"), 7, Duration::from_secs(10)).unwrap_err();
        assert!(err.to_string().contains("never reached `boot`"), "{err}");
    }

    #[test]
    fn a_target_ended_by_a_signal_is_an_error() {
        let err = ask(sh(), &shell("kill -9 $$"), 7, Duration::from_secs(10)).unwrap_err();
        assert!(err.to_string().contains("signal"), "{err}");
    }

    #[test]
    fn a_target_that_never_answers_is_killed_after_the_timeout() {
        let started = Instant::now();
        let err = ask(sh(), &shell("exec sleep 30"), 7, Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn a_target_that_cannot_be_started_is_an_error() {
        let err = ask(
            Path::new("/nonexistent/binary"),
            &shell(""),
            7,
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
