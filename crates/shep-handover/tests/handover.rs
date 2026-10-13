//! End to end: a process replaces its own image and the new one carries on.
//!
//! `execve` replaces the test process, so each test that really execs is
//! three stages of this one test binary. The ordinary run re-runs the test in
//! a child with a stage marker; that child hands over to itself, and the image
//! it execs into is this binary again, started with the same arguments, which
//! finds it is a successor through `boot`.

#[cfg(unix)]
mod unix {
    use std::io::Read as _;
    use std::os::fd::AsRawFd as _;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use nix::fcntl::{FcntlArg, FdFlag, fcntl};
    use serde::{Deserialize, Serialize};
    use shep_handover::{Arrival, BootError, HandOverError, Handover, State, Verdict};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    const STAGE: &str = "SHEP_HANDOVER_TEST_STAGE";

    /// Run `test` as the old image in a child process and return its stdout,
    /// which the old image and its successor share.
    fn run_old_image(test: &str) -> String {
        let out = Command::new(std::env::current_exe().unwrap())
            .args([&format!("unix::{test}"), "--exact", "--nocapture"])
            .env(STAGE, "old")
            .env_remove(shep_handover::HANDOVER_ENV)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{stdout}{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // `--exact` against a stale name matches nothing: the child would run
        // zero tests, exit successfully and print no marker.
        assert!(
            stdout.contains("running 1 test"),
            "the child ran no test, so `{test}` is not a test name any more: {stdout}"
        );
        stdout
    }

    fn is_old_image() -> bool {
        std::env::var_os(STAGE).is_some()
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    fn close_on_exec(fd: &impl std::os::fd::AsRawFd) -> bool {
        let flags = fcntl(fd.as_raw_fd(), FcntlArg::F_GETFD).unwrap();
        FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC)
    }

    #[derive(Serialize, Deserialize)]
    struct Stream {
        child: u32,
        seen: Vec<String>,
    }

    impl State for Stream {
        const FORMAT: u32 = 1;
    }

    #[test]
    fn a_child_streaming_through_a_pipe_survives_a_handover_to_itself() {
        if let Some(arrival) = shep_handover::boot::<Stream>().unwrap() {
            stream_successor(arrival);
        }
        if is_old_image() {
            stream_old_image();
        }

        let stdout =
            run_old_image("a_child_streaming_through_a_pipe_survives_a_handover_to_itself");
        assert!(stdout.contains(r#"seen before: ["one"]"#), "{stdout}");
        assert!(stdout.contains(r#"rest: "two\n""#), "{stdout}");
        assert!(
            stdout.contains("exit: ChildExit { code: Some(7), signal: None }"),
            "{stdout}"
        );
    }

    /// The child prints a line, waits for one back, prints another and exits
    /// 7; the handover happens while it waits.
    #[allow(clippy::zombie_processes, reason = "the successor reaps the child")]
    fn stream_old_image() -> ! {
        let mut child = Command::new("sh")
            .args(["-c", "echo one; read go; echo two; exit 7"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = child.stdout.take().unwrap();
        let stdin = child.stdin.take().unwrap();

        // A byte at a time, so nothing past the first line is buffered here.
        let mut line = Vec::new();
        let mut byte = [0_u8; 1];
        while byte[0] != b'\n' {
            out.read_exact(&mut byte).unwrap();
            line.push(byte[0]);
        }
        let seen = vec![String::from_utf8(line).unwrap().trim_end().to_owned()];

        let state = Stream {
            child: child.id(),
            seen,
        };
        let err = Handover::new(&state)
            .carry("child-out", &out)
            .carry("child-in", &stdin)
            .exec(&|_: &Path| Verdict::Carryable)
            .unwrap_err();
        panic!("the exec should not have returned: {err}");
    }

    fn stream_successor(mut arrival: Arrival<Stream>) -> ! {
        runtime().block_on(async {
            let mut reader = arrival
                .fds
                .take("child-out")
                .unwrap()
                .into_reader()
                .unwrap();
            let mut writer = arrival.fds.take("child-in").unwrap().into_writer().unwrap();
            writer.write_all(b"go\n").await.unwrap();
            drop(writer);

            let mut rest = String::new();
            reader.read_to_string(&mut rest).await.unwrap();
            let exit = shep_handover::wait_for_child(arrival.state.child).await;

            println!("seen before: {:?}", arrival.state.seen);
            println!("rest: {rest:?}");
            println!("exit: {:?}", exit.unwrap());
        });
        std::process::exit(0);
    }

    #[derive(Serialize, Deserialize)]
    struct Probe {
        uncarried: i32,
        carried: i32,
    }

    impl State for Probe {
        const FORMAT: u32 = 1;
    }

    #[test]
    fn a_descriptor_not_named_to_carry_is_closed_in_the_new_image() {
        // Checked before anything else runs in the successor: a number this
        // image opens later could be the one the old image never carried.
        if let Some(arrival) = shep_handover::boot::<Probe>().unwrap() {
            let open = |fd: i32| fcntl(fd, FcntlArg::F_GETFD).is_ok();
            println!("carried open: {}", open(arrival.state.carried));
            println!("uncarried open: {}", open(arrival.state.uncarried));
            std::process::exit(0);
        }
        if is_old_image() {
            let (carried, _w1) = std::io::pipe().unwrap();
            let (uncarried, _w2) = std::io::pipe().unwrap();
            let state = Probe {
                uncarried: uncarried.as_raw_fd(),
                carried: carried.as_raw_fd(),
            };
            let err = Handover::new(&state)
                .carry("kept", &carried)
                .exec(&|_: &Path| Verdict::Carryable)
                .unwrap_err();
            panic!("the exec should not have returned: {err}");
        }

        let stdout = run_old_image("a_descriptor_not_named_to_carry_is_closed_in_the_new_image");
        assert!(stdout.contains("carried open: true"), "{stdout}");
        assert!(stdout.contains("uncarried open: false"), "{stdout}");
    }

    #[derive(Serialize, Deserialize)]
    struct Newer {
        note: String,
    }

    impl State for Newer {
        const FORMAT: u32 = 2;
    }

    #[derive(Serialize, Deserialize)]
    struct Older {
        note: String,
    }

    impl State for Older {
        const FORMAT: u32 = 1;
    }

    #[test]
    fn a_target_that_cannot_read_the_format_is_refused_and_the_old_image_keeps_running() {
        // First thing, as a target must: when the old image probes this
        // binary, this exits with the answer and goes no further.
        assert!(shep_handover::boot::<Older>().unwrap().is_none());
        if is_old_image() {
            let state = Newer {
                note: "from the future".to_owned(),
            };
            let err = Handover::new(&state)
                .exec(&|_: &Path| Verdict::Carryable)
                .unwrap_err();
            assert!(
                matches!(err, HandOverError::Format { format: 2, .. }),
                "{err}"
            );
            println!(
                "refused: {}",
                err.to_string().replace(
                    &std::env::current_exe().unwrap().display().to_string(),
                    "<this binary>",
                )
            );
            println!("still running");
            return;
        }

        let stdout = run_old_image(
            "a_target_that_cannot_read_the_format_is_refused_and_the_old_image_keeps_running",
        );
        assert!(
            stdout.contains("refused: the target <this binary> does not read state format 2"),
            "{stdout}"
        );
        assert!(stdout.contains("still running"), "{stdout}");
    }

    #[test]
    fn an_unprobed_state_in_a_format_the_new_build_cannot_read_is_refused_by_boot() {
        if std::env::var_os(shep_handover::HANDOVER_ENV).is_some() {
            let refusal = shep_handover::boot::<Older>().unwrap_err();
            assert!(matches!(
                refusal,
                BootError::Format {
                    found: Some(2),
                    supported: 1
                }
            ));
            println!("refused: {refusal}");
            std::process::exit(0);
        }
        if is_old_image() {
            let state = Newer {
                note: "from the future".to_owned(),
            };
            let err = Handover::new(&state)
                .unprobed()
                .exec(&|_: &Path| Verdict::Carryable)
                .unwrap_err();
            panic!("the exec should not have returned: {err}");
        }

        let stdout = run_old_image(
            "an_unprobed_state_in_a_format_the_new_build_cannot_read_is_refused_by_boot",
        );
        assert!(
            stdout
                .contains("refused: the handover state is format 2, and this build reads format 1"),
            "{stdout}"
        );
    }

    #[derive(Serialize, Deserialize)]
    struct Nothing {}

    impl State for Nothing {
        const FORMAT: u32 = 1;
    }

    #[test]
    fn a_refused_gate_leaves_the_image_running_and_returns_its_reason() {
        let dir = tempfile::tempdir().unwrap();
        let (read, _write) = std::io::pipe().unwrap();

        let err = Handover::new(&Nothing {})
            .carry("pipe", &read)
            .blob_dir(dir.path())
            .exec(&|_: &Path| {
                Verdict::Refused("a pump is wedged; stop and start instead".to_owned())
            })
            .unwrap_err();

        assert!(
            matches!(&err, HandOverError::Refused { reason } if reason == "a pump is wedged; stop and start instead"),
            "{err}"
        );
        assert!(close_on_exec(&read), "a refusal clears nothing");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_failed_exec_leaves_the_image_running_with_nothing_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let not_executable = dir.path().join("not-a-binary");
        std::fs::write(&not_executable, "this will never execute").unwrap();
        let (read, _write) = std::io::pipe().unwrap();

        let err = Handover::new(&Nothing {})
            .carry("pipe", &read)
            .target(&not_executable)
            .unprobed()
            .blob_dir(dir.path())
            .exec(&|_: &Path| Verdict::Carryable)
            .unwrap_err();

        assert!(matches!(err, HandOverError::Exec(_)), "{err}");
        assert!(
            close_on_exec(&read),
            "the descriptor is close-on-exec again"
        );
        let left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["not-a-binary"], "the blob was removed");
    }
}

#[cfg(not(unix))]
mod unsupported {
    use shep_handover::{BootError, HandOverError, Handover, State, Verdict};

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Nothing {}

    impl State for Nothing {
        const FORMAT: u32 = 1;
    }

    #[test]
    fn handing_over_is_not_supported() {
        let err = Handover::new(&Nothing {})
            .exec(&|_: &std::path::Path| Verdict::Carryable)
            .unwrap_err();
        assert!(matches!(err, HandOverError::Unsupported), "{err}");
        assert_eq!(
            err.to_string(),
            "handing over is not supported on this platform"
        );
    }

    #[test]
    fn booting_is_not_supported() {
        let err = shep_handover::boot::<Nothing>().unwrap_err();
        assert!(matches!(err, BootError::Unsupported), "{err}");
    }

    #[tokio::test]
    async fn waiting_for_a_child_is_not_supported() {
        let err = shep_handover::wait_for_child(1).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    }
}
