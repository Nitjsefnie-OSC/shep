//! `Request::TailLog`: the end of one sheep's log file, read by the shepherd
//! so a client on another host need not reach its disk.

use super::*;
use shep_core::protocol::{LogStream, LogTail, ProcessInfo};
use std::path::Path;

/// The shepherd's stamp, spelled out so the fixture is independent of the
/// code that writes and strips it.
const STAMP: &str = "2026-09-02T14:22:31.412+02:00 ";

/// Starts `web` with its out and err logs in `dir` and returns its id.
async fn start_logging_web(h: &Harness, dir: &Path, stamped: bool) -> u32 {
    let mut app = AppConfig::minimal("web", "./srv");
    app.out_file = Some(dir.join("web-out.log").display().to_string());
    app.err_file = Some(dir.join("web-err.log").display().to_string());
    app.log_timestamps = stamped;
    let started = reply_of(dispatch(envelope(1, Request::Start { apps: vec![app] }), &h.ctx).await);
    assert!(started.result.is_ok(), "{:?}", started.result);
    list_flock(&h.ctx, 2).await[0].id
}

async fn tail(h: &Harness, id: u32, stream: LogStream, lines: u32) -> Reply {
    reply_of(dispatch(envelope(3, Request::TailLog { id, stream, lines }), &h.ctx).await)
}

#[tokio::test(start_paused = true)]
async fn the_tail_of_a_known_sheep_is_returned_oldest_first() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(vec![ProcScript::never_exits()]);
    let id = start_logging_web(&h, dir.path(), false).await;
    std::fs::write(dir.path().join("web-out.log"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.path().join("web-err.log"), "boom\n").unwrap();

    let out = tail(&h, id, LogStream::Out, 2).await;
    let err = tail(&h, id, LogStream::Err, 2).await;

    assert_eq!(
        out.result.unwrap(),
        Response::LogTail(LogTail::Lines {
            lines: vec!["two".to_string(), "three".to_string()],
            truncated: true,
        })
    );
    assert_eq!(
        err.result.unwrap(),
        Response::LogTail(LogTail::Lines {
            lines: vec!["boom".to_string()],
            truncated: false,
        })
    );
}

#[tokio::test(start_paused = true)]
async fn an_unknown_id_is_not_found() {
    let h = harness(vec![]);

    let reply = tail(&h, 99, LogStream::Out, 10).await;

    assert_eq!(reply.result.unwrap_err().code, RpcErrorCode::NotFound);
}

/// A row from a shepherd that reported no path. Built by hand because the
/// supervisor always fills both in.
#[tokio::test]
async fn a_sheep_with_no_err_path_answers_no_path() {
    let row = ProcessInfo::builder(1, "web".to_string(), ProcStatus::Online)
        .out_file(Some("/logs/web-out.log".to_string()))
        .err_file(None)
        .build();

    let answer = super::super::log_tail::tail_row(&row, LogStream::Err, 10)
        .await
        .unwrap();

    assert_eq!(answer, LogTail::NoPath);
}

#[tokio::test(start_paused = true)]
async fn a_missing_file_answers_missing_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(vec![ProcScript::never_exits()]);
    let id = start_logging_web(&h, dir.path(), true).await;

    let reply = tail(&h, id, LogStream::Out, 10).await;

    assert_eq!(
        reply.result.unwrap(),
        Response::LogTail(LogTail::Missing {
            path: dir.path().join("web-out.log").display().to_string(),
        })
    );
}

/// A directory where the file should be, so the open succeeds and the read
/// fails, on every platform and without depending on who runs the test.
#[tokio::test(start_paused = true)]
async fn an_unreadable_file_answers_unreadable_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(vec![ProcScript::never_exits()]);
    let id = start_logging_web(&h, dir.path(), true).await;
    std::fs::create_dir(dir.path().join("web-out.log")).unwrap();

    let reply = tail(&h, id, LogStream::Out, 10).await;

    let Ok(Response::LogTail(LogTail::Unreadable { path, reason })) = reply.result else {
        panic!("expected Unreadable, got {:?}", reply.result)
    };
    assert_eq!(path, dir.path().join("web-out.log").display().to_string());
    assert!(!reason.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_stamped_sheep_is_returned_without_its_stamps() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(vec![ProcScript::never_exits()]);
    let id = start_logging_web(&h, dir.path(), true).await;
    std::fs::write(
        dir.path().join("web-out.log"),
        format!("{STAMP}listening\n{STAMP}ready\n"),
    )
    .unwrap();

    let reply = tail(&h, id, LogStream::Out, 10).await;

    assert_eq!(
        reply.result.unwrap(),
        Response::LogTail(LogTail::Lines {
            lines: vec!["listening".to_string(), "ready".to_string()],
            truncated: false,
        })
    );
}

/// An app that turned the stamp off wrote its own prefix, and the shepherd
/// must not cut into it.
#[tokio::test(start_paused = true)]
async fn an_unstamped_sheep_is_returned_as_written() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(vec![ProcScript::never_exits()]);
    let id = start_logging_web(&h, dir.path(), false).await;
    let own = format!("{STAMP}the app's own prefix");
    std::fs::write(dir.path().join("web-out.log"), format!("{own}\n")).unwrap();

    let reply = tail(&h, id, LogStream::Out, 10).await;

    assert_eq!(
        reply.result.unwrap(),
        Response::LogTail(LogTail::Lines {
            lines: vec![own],
            truncated: false,
        })
    );
}

/// The request names a sheep and a stream. A path riding along on the wire
/// is not a field of it, so it cannot choose the file.
#[tokio::test(start_paused = true)]
async fn the_request_carries_no_path_the_shepherd_could_be_made_to_read() {
    let dir = tempfile::tempdir().unwrap();
    let h = harness(vec![ProcScript::never_exits()]);
    let id = start_logging_web(&h, dir.path(), false).await;
    std::fs::write(dir.path().join("web-out.log"), "the sheep's own\n").unwrap();
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "not for the wire\n").unwrap();
    let wire = serde_json::json!({
        "id": 4,
        "deadline_ms": null,
        "body": {
            "kind": "tail_log",
            "id": id,
            "stream": "out",
            "lines": 10,
            "path": secret,
            "out_file": secret,
        },
    });
    let envelope: Envelope = serde_json::from_value(wire).unwrap();

    let reply = reply_of(dispatch(envelope, &h.ctx).await);

    assert_eq!(
        reply.result.unwrap(),
        Response::LogTail(LogTail::Lines {
            lines: vec!["the sheep's own".to_string()],
            truncated: false,
        })
    );
}

/// What an older shepherd does with `tail_log`: it decodes the kind it has
/// never heard of as `Unrecognized`, refuses by name, and goes on serving.
#[tokio::test(start_paused = true)]
async fn an_older_shepherds_unrecognized_request_answers_unsupported() {
    let h = harness(vec![]);
    let wire = r#"{"id":5,"deadline_ms":null,"body":{"kind":"tail_log_from_the_future"}}"#;
    let envelope: Envelope = serde_json::from_str(wire).unwrap();
    assert_eq!(envelope.body, Request::Unrecognized);

    let refused = reply_of(dispatch(envelope, &h.ctx).await);
    let next = reply_of(dispatch(super::envelope(6, Request::Ping), &h.ctx).await);

    assert_eq!(refused.id, 5);
    assert_eq!(refused.result.unwrap_err().code, RpcErrorCode::Unsupported);
    assert_eq!(next.result.unwrap(), Response::Pong);
}
