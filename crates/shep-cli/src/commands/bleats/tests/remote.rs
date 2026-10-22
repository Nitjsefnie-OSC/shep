//! The tail read through a host's shepherd: `Request::TailLog` in place of
//! `read_tail`, with everything else `bleats` does left shared. Each test
//! hands `bleats_over` a `Client` on a fake shepherd, which is all the
//! source needs, so none of them dials ssh.

use std::sync::Arc;

use shep_core::protocol::{LogStream, LogTail, Request, Response, RpcError, RpcErrorCode};
use tokio::sync::Notify;

use super::*;

/// The shepherd's stamp, which `read_tail` takes off a stamped sheep's lines.
const STAMP: &str = "2026-09-02T14:22:31.412+02:00 ";

/// The host every run below names.
const HOST: &str = "amp2";

/// What one run of `bleats_over` did.
struct Run {
    code: ExitCode,
    out: String,
    err: String,
    /// Every request the fake shepherd received, in order.
    requests: Vec<Request>,
}

impl Run {
    /// The `TailLog` requests among [`Self::requests`], as `(id, stream, lines)`.
    fn tails(&self) -> Vec<(u32, LogStream, u32)> {
        self.requests
            .iter()
            .filter_map(|request| match request {
                Request::TailLog { id, stream, lines } => Some((*id, *stream, *lines)),
                _ => None,
            })
            .collect()
    }
}

/// Runs `bleats_over` against a fake shepherd that lists `rows` and serves
/// each `TailLog` from `serve`. `host` is `None` for a local run, which
/// reads the files `rows` name and never sends a `TailLog`.
///
/// A follow is interrupted once the fake has seen its `Subscribe`.
async fn run(
    host: Option<&str>,
    format: Format,
    rows: &[ProcessInfo],
    args: &BleatsArgs,
    serve: impl Fn(u32, LogStream, u32) -> Result<LogTail, RpcError> + Send + 'static,
) -> Run {
    let mut err = Vec::new();
    let mut run = run_into(host, format, rows, args, serve, &mut err).await;
    run.err = String::from_utf8(err).unwrap();
    run
}

/// [`run`] with the stderr writer chosen by the caller. The returned
/// [`Run::err`] is empty: the caller owns what was written.
async fn run_into(
    host: Option<&str>,
    format: Format,
    rows: &[ProcessInfo],
    args: &BleatsArgs,
    serve: impl Fn(u32, LogStream, u32) -> Result<LogTail, RpcError> + Send + 'static,
    err: &mut dyn io::Write,
) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let subscribed = Arc::new(Notify::new());
    let listing = rows.to_vec();
    let on_subscribe = Arc::clone(&subscribed);
    let (client, mut requests) =
        shep_client::testing::fake_client_answering_refusals(&path, move |request| match request {
            Request::ListFlock => Ok(Response::Flock(listing.clone())),
            Request::TailLog { id, stream, lines } => {
                serve(*id, *stream, *lines).map(Response::LogTail)
            }
            Request::Subscribe { .. } => {
                on_subscribe.notify_one();
                Ok(Response::Subscribed)
            }
            _ => Ok(Response::Pong),
        })
        .await;

    let follows = !args.no_follow;
    let interrupt = async move {
        if follows {
            subscribed.notified().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    let mut out = Vec::new();
    let code = {
        let mut streams = Streams {
            out: &mut out,
            err,
            style: crate::style::Presentation::BARE,
            fmt: format,
        };
        tokio::time::timeout(
            RUN_TIMEOUT,
            bleats_over(&client, &mut streams, false, args, host, interrupt),
        )
        .await
        .expect("a fake shepherd answers at once")
    };
    let mut seen = Vec::new();
    while let Ok(envelope) = requests.try_recv() {
        seen.push(envelope.body);
    }
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::new(),
        requests: seen,
    }
}

/// A shepherd that has these files on its disk: it serves the same
/// `read_tail` a local `bleats` calls, honouring the line count asked for.
fn serving_files(
    rows: &[ProcessInfo],
) -> impl Fn(u32, LogStream, u32) -> Result<LogTail, RpcError> + Send + 'static + use<> {
    let rows = rows.to_vec();
    move |id, stream, lines| {
        let row = rows.iter().find(|row| row.id == id).unwrap();
        let path = match stream {
            LogStream::Err => row.err_file.clone(),
            _ => row.out_file.clone(),
        };
        let Some(path) = path else {
            return Ok(LogTail::NoPath);
        };
        let stamped = row.log_timestamps.unwrap_or(true);
        Ok(
            match read_tail(Path::new(&path), usize::try_from(lines).unwrap(), stamped) {
                Ok((lines, truncated)) => LogTail::Lines { lines, truncated },
                Err(err) if err.kind() == io::ErrorKind::NotFound => LogTail::Missing { path },
                Err(err) => LogTail::Unreadable {
                    path,
                    reason: err.to_string(),
                },
            },
        )
    }
}

/// Two instances of `web` and one `solo`, each with an out and an err file
/// on disk, stamped the way the shepherd writes them.
fn flock_on_disk(dir: &Path) -> Vec<ProcessInfo> {
    let mut rows = Vec::new();
    for (id, app, slot) in [(1, "web", Some(0)), (2, "web", Some(1)), (3, "solo", None)] {
        let stem = format!("{app}-{id}");
        let lines = |stream: &str| {
            (1..=4)
                .map(|n| format!("{STAMP}{app} {id} {stream} line {n}\n"))
                .collect::<String>()
        };
        let mut row = info(id, app);
        row.instance = slot;
        row.out_file = Some(write_log(dir, &format!("{stem}-out.log"), &lines("out")));
        row.err_file = Some(write_log(dir, &format!("{stem}-err.log"), &lines("err")));
        rows.push(row);
    }
    rows
}

/// A single `web` row whose files the fake serves from `serve`, not a disk.
fn web() -> Vec<ProcessInfo> {
    vec![info(1, "web")]
}

fn refusing(code: RpcErrorCode) -> RpcError {
    RpcError {
        code,
        message: "no".to_owned(),
        daemon_version: None,
    }
}

#[tokio::test]
async fn a_remote_tail_prints_the_same_lines_and_labels_as_a_local_one() {
    let dir = tempfile::tempdir().unwrap();
    let rows = flock_on_disk(dir.path());
    let args = no_follow_args("all");

    for format in [Format::Table, Format::Json] {
        let local = run(None, format, &rows, &args, serving_files(&rows)).await;
        let remote = run(Some(HOST), format, &rows, &args, serving_files(&rows)).await;

        assert_eq!(local.code, ExitCode::Success);
        assert_eq!(remote.code, ExitCode::Success);
        let labelled = match format {
            Format::Table => "web:1 | web 2 out line 1",
            Format::Json => r#""name":"web","stream":"out","instance":1,"line":"web 2 out line 1""#,
        };
        assert!(
            local.out.contains(labelled) && local.out.contains("solo"),
            "the fixture must hold a labelled and a bare sheep: {}",
            local.out
        );
        assert_eq!(remote.out, local.out, "{format:?}");
        assert_eq!(remote.err, local.err, "{format:?}");
        assert!(
            local.tails().is_empty(),
            "a local run reads files, not the wire"
        );
        assert_eq!(remote.tails().len(), 6, "one request per file");
    }
}

#[tokio::test]
async fn a_remote_tail_of_a_shared_log_prints_it_once_without_a_slot_label() {
    let rows: Vec<ProcessInfo> = (0..2)
        .map(|slot| {
            let mut row = info_with_instance(slot, "talker", slot);
            row.out_file = Some("/srv/talker-out.log".to_owned());
            row
        })
        .collect();

    let remote = run(
        Some(HOST),
        Format::Table,
        &rows,
        &no_follow_args_out("talker"),
        |_, _, _| {
            Ok(LogTail::Lines {
                lines: vec!["line one".to_owned()],
                truncated: false,
            })
        },
    )
    .await;

    assert_eq!(remote.out, "talker | line one\n");
    assert_eq!(remote.tails().len(), 1, "one file, one request");
}

#[tokio::test]
async fn a_remote_missing_log_is_a_log_missing_notice_and_the_exit_stays_success() {
    let remote = run(
        Some(HOST),
        Format::Table,
        &web(),
        &no_follow_args_out("web"),
        |_, _, _| {
            Ok(LogTail::Missing {
                path: "/logs/web-0-out.log".to_owned(),
            })
        },
    )
    .await;

    assert_eq!(remote.code, ExitCode::Success);
    assert_eq!(
        remote.err,
        "notice[log_missing]: web: no out log at /logs/web-0-out.log on amp2 yet\n"
    );
    assert_eq!(remote.out, "");
}

#[tokio::test]
async fn a_remote_unreadable_log_is_a_log_unreadable_notice_and_exits_failure() {
    let remote = run(
        Some(HOST),
        Format::Table,
        &web(),
        &no_follow_args_out("web"),
        |_, _, _| {
            Ok(LogTail::Unreadable {
                path: "/logs/web-0-out.log".to_owned(),
                reason: "permission denied".to_owned(),
            })
        },
    )
    .await;

    assert_eq!(remote.code, ExitCode::Failure);
    assert_eq!(
        remote.err,
        "notice[log_unreadable]: failed to read /logs/web-0-out.log on amp2: permission denied\n"
    );
}

#[tokio::test]
async fn an_answer_this_client_does_not_understand_is_a_log_unreadable_notice() {
    let remote = run(
        Some(HOST),
        Format::Table,
        &web(),
        &no_follow_args_out("web"),
        |_, _, _| Ok(LogTail::Unrecognized),
    )
    .await;

    assert_eq!(remote.code, ExitCode::Failure);
    assert!(
        remote.err.starts_with("notice[log_unreadable]:")
            && remote.err.contains("does not understand the answer"),
        "{}",
        remote.err
    );
}

#[tokio::test]
async fn a_remote_relative_path_is_not_resolved_against_the_local_directory() {
    let mut rows = web();
    rows[0].out_file = Some("logs/web-out.log".to_owned());

    let remote = run(
        Some(HOST),
        Format::Table,
        &rows,
        &no_follow_args_out("web"),
        |_, _, _| {
            Ok(LogTail::Missing {
                path: "logs/web-out.log".to_owned(),
            })
        },
    )
    .await;

    assert_eq!(
        remote.err,
        "notice[log_missing]: web: no out log at logs/web-out.log on amp2 yet\n"
    );
    let here = std::env::current_dir().unwrap();
    assert!(
        !remote.err.contains(here.to_str().unwrap()),
        "this machine's directory names nothing on the host: {}",
        remote.err
    );
}

#[tokio::test]
async fn an_unsupported_answer_fails_once_naming_the_host_and_the_remedy() {
    let mut rows = flock_on_disk(tempfile::tempdir().unwrap().path());
    rows.truncate(2);

    let remote = run(
        Some(HOST),
        Format::Table,
        &rows,
        &no_follow_args("all"),
        |_, _, _| Err(refusing(RpcErrorCode::Unsupported)),
    )
    .await;

    assert_eq!(remote.code, ExitCode::Unsupported);
    assert_eq!(
        remote.err.lines().count(),
        1,
        "one failure for the verb, not one per file: {}",
        remote.err
    );
    assert!(remote.err.contains("amp2"), "{}", remote.err);
    assert!(
        remote.err.contains("predates remote log reads"),
        "{}",
        remote.err
    );
    assert!(
        remote.err.contains("ssh amp2 shep daemon reload"),
        "{}",
        remote.err
    );
    assert_eq!(remote.tails().len(), 1, "the first refusal ends the reads");
    assert_eq!(remote.out, "");
}

/// A stderr that holds what it is given until it is flushed, as a buffered
/// one does.
#[derive(Default)]
struct Buffered {
    pending: Vec<u8>,
    flushed: Vec<u8>,
}

impl io::Write for Buffered {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushed.append(&mut self.pending);
        Ok(())
    }
}

#[tokio::test]
async fn the_failure_that_stops_a_remote_read_is_flushed_to_stderr() {
    let rows = web();
    let mut err = Buffered::default();

    let remote = run_into(
        Some(HOST),
        Format::Table,
        &rows,
        &no_follow_args("all"),
        |_, _, _| Err(refusing(RpcErrorCode::Unsupported)),
        &mut err,
    )
    .await;

    assert_eq!(remote.code, ExitCode::Unsupported);
    let flushed = String::from_utf8(err.flushed).unwrap();
    assert!(flushed.contains("ssh amp2 shep daemon reload"), "{flushed}");
    assert!(err.pending.is_empty(), "nothing left unflushed");
}

#[tokio::test]
async fn follow_under_host_prints_the_backlog_through_the_remote_read_then_follows() {
    let dir = tempfile::tempdir().unwrap();
    // Rows whose files exist here, and hold text the host never wrote.
    let mut rows = web();
    rows[0].out_file = Some(write_log(dir.path(), "web-out.log", "from-this-machine\n"));
    let args = bleats_args("all", false, true, false);
    assert!(args.lines > 0, "the default asks for a backlog");

    let remote = run(Some(HOST), Format::Table, &rows, &args, |_, _, _| {
        Ok(LogTail::Lines {
            lines: vec!["from-the-host".to_owned()],
            truncated: false,
        })
    })
    .await;

    assert_eq!(remote.code, ExitCode::Success);
    assert_eq!(remote.out, "web | from-the-host\n");
    assert_eq!(remote.err, "", "a host backlog needs no notice");
    assert!(
        matches!(
            remote.requests.as_slice(),
            [
                Request::ListFlock,
                Request::TailLog { .. },
                Request::Subscribe { .. }
            ]
        ),
        "the tail comes between the listing and the subscribe: {:?}",
        remote.requests
    );
}

#[tokio::test]
async fn follow_under_host_stops_on_an_unsupported_shepherd_without_subscribing() {
    let remote = run(
        Some(HOST),
        Format::Table,
        &web(),
        &bleats_args("all", false, false, false),
        |_, _, _| Err(refusing(RpcErrorCode::Unsupported)),
    )
    .await;

    assert_eq!(remote.code, ExitCode::Unsupported);
    assert!(
        !remote
            .requests
            .iter()
            .any(|request| matches!(request, Request::Subscribe { .. })),
        "{:?}",
        remote.requests
    );
}

#[tokio::test]
async fn lines_out_and_err_flags_choose_the_same_requests_locally_and_remotely() {
    let dir = tempfile::tempdir().unwrap();
    let rows = flock_on_disk(dir.path());
    let two_lines_of_solo = |mut args: BleatsArgs| {
        args.selector = "solo".to_owned();
        args.lines = 2;
        args
    };

    for (args, expected) in [
        (no_follow_args_out("solo"), vec![(3, LogStream::Out, 2)]),
        (no_follow_args_err("solo"), vec![(3, LogStream::Err, 2)]),
        (
            no_follow_args("solo"),
            vec![(3, LogStream::Out, 2), (3, LogStream::Err, 2)],
        ),
    ] {
        let args = two_lines_of_solo(args);
        let local = run(None, Format::Table, &rows, &args, serving_files(&rows)).await;
        let remote = run(
            Some(HOST),
            Format::Table,
            &rows,
            &args,
            serving_files(&rows),
        )
        .await;

        assert_eq!(remote.tails(), expected);
        assert_eq!(remote.out, local.out);
        assert_eq!(
            local.out.lines().count(),
            expected.len() * 2,
            "-n 2 shows two lines of each file read: {}",
            local.out
        );
    }
}
