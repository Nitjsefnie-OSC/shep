//! A follow over `--host`: how it ends when the link and the operator's
//! Ctrl+C arrive together. `bleats_over` takes the host as an argument, and a
//! fake shepherd stands in for the ssh hop, which cannot be scripted here.

use super::*;

/// Runs `bleats_over` as a host follow against a fake shepherd that lists one
/// sheep, `web`, then pushes `live` and closes. Answers the exit code, stdout
/// and stderr.
async fn host_follow(
    quiet: bool,
    sheep: ProcessInfo,
    live: Option<&str>,
    args: &BleatsArgs,
    interrupt: impl std::future::Future<Output = ()> + Send,
) -> (ExitCode, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, daemon) = fake_client_with_push(&path).await;
    daemon.reply_to_list(vec![sheep]);
    if let Some(line) = live {
        daemon
            .push(BusEvent::LogOut {
                id: 1,
                line: line.into(),
            })
            .await;
    }
    daemon.close_after_subscribe().await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        tokio::time::timeout(
            RUN_TIMEOUT,
            bleats_over(&client, &mut streams, quiet, args, Some("amp2"), interrupt),
        )
        .await
        .expect("the grace ends the follow")
    };
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

/// The ssh child dies on the same SIGINT as this process, so the stream can
/// end a moment before the interrupt is delivered.
#[tokio::test]
async fn a_stream_that_ends_just_before_an_interrupt_exits_success_over_a_host() {
    let late_interrupt = async { tokio::time::sleep(Duration::from_millis(50)).await };

    let (code, ..) = host_follow(
        false,
        info(1, "web"),
        None,
        &follow_args("all"),
        late_interrupt,
    )
    .await;

    assert_eq!(code, ExitCode::Success);
}

/// A lost link with no interrupt behind it is still a lost shepherd, once
/// the grace has passed.
#[tokio::test]
async fn a_stream_that_ends_alone_over_a_host_is_still_daemon_unreachable_after_the_grace() {
    let started = std::time::Instant::now();

    let (code, ..) = host_follow(
        false,
        info(1, "web"),
        None,
        &follow_args("all"),
        std::future::pending(),
    )
    .await;

    assert_eq!(code, ExitCode::DaemonUnreachable);
    assert!(started.elapsed() >= crate::remote::HOST_INTERRUPT_GRACE);
}

/// Runs a host follow against a shepherd that answers the one listing and
/// then drops the connection: `serve_one_request` ends after its reply, so
/// the `Request::Subscribe` that follows it is the request that gets no
/// answer, every time. That is the shape a Ctrl+C takes over a host: the ssh
/// child dies on the same signal, and the link goes with it.
async fn host_follow_over_a_link_that_closes(
    interrupt: impl std::future::Future<Output = ()> + Send,
) -> (ExitCode, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let _listing = shep_client::testing::serve_one_request(
        &path,
        shep_client::testing::sample_ack(),
        shep_core::protocol::Response::Flock(vec![info(1, "web")]),
    )
    .await;
    let client = shep_client::Client::connect(&path).await.unwrap();

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        tokio::time::timeout(
            RUN_TIMEOUT,
            bleats_over(
                &client,
                &mut streams,
                false,
                &follow_args("all"),
                Some("amp2"),
                interrupt,
            ),
        )
        .await
        .expect("the link closes, and the interrupt or the grace ends the follow")
    };
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

/// The interrupt arrives a moment after the link goes, which is how a real
/// Ctrl+C lands: the signal kills the ssh child first. Nothing is printed,
/// the same as an interrupt that ends a follow already subscribed.
#[tokio::test]
async fn a_host_link_that_closes_mid_subscribe_under_an_interrupt_exits_success_and_says_nothing() {
    let late_interrupt = async { tokio::time::sleep(Duration::from_millis(50)).await };

    let (code, out, err) = host_follow_over_a_link_that_closes(late_interrupt).await;

    assert_eq!(code, ExitCode::Success);
    assert_eq!(out, "");
    assert_eq!(err, "", "a Ctrl+C the operator meant is no lost shepherd");
}

/// The same closed link with no interrupt behind it is still a shepherd that
/// is gone, once the grace has passed.
#[tokio::test]
async fn a_host_link_that_closes_mid_subscribe_with_no_interrupt_is_a_lost_shepherd() {
    let started = std::time::Instant::now();

    let (code, _, err) = host_follow_over_a_link_that_closes(std::future::pending()).await;

    assert_eq!(code, ExitCode::DaemonUnreachable);
    assert!(
        err.starts_with("error[daemon_unreachable]"),
        "a link nobody interrupted is still reported: {err}"
    );
    assert!(started.elapsed() >= crate::remote::HOST_INTERRUPT_GRACE);
}

/// A shepherd that answers a request with a refusal is answering, whatever
/// the operator pressed at the same moment: only a link that closed gets the
/// grace, and an interrupt ready before the answer must not swallow it.
#[tokio::test]
async fn a_refusal_over_a_host_is_not_read_as_the_operators_interrupt() {
    let dir = tempfile::tempdir().unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, _daemon) = shep_client::testing::fake_client_replying_err(
        &path,
        shep_core::protocol::RpcErrorCode::Unsupported,
        "no",
    )
    .await;

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        tokio::time::timeout(
            RUN_TIMEOUT,
            bleats_over(
                &client,
                &mut streams,
                false,
                &follow_args("all"),
                Some("amp2"),
                async {},
            ),
        )
        .await
        .expect("a refusal answers at once")
    };
    let err = String::from_utf8(err).unwrap();

    assert_eq!(code, ExitCode::Unsupported);
    assert!(err.starts_with("error[unsupported]"), "{err}");
}

/// The grace path prints no error, and the backlog a tail already wrote is
/// still in the writer's buffer. `Streams::out` is whatever the caller passed,
/// so reaching it is this module's job on every way out.
#[tokio::test]
async fn a_host_link_that_closes_under_an_interrupt_flushes_the_backlog_it_printed() {
    let mut sink: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let (code, reached) = {
        let mut buffered = std::io::BufWriter::new(&mut sink);
        let mut streams = Streams {
            out: &mut buffered,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        write_line(&mut streams, 1, "web", None, "out", "backlog")
            .expect("a Vec never fails a write");
        let mut interrupt = std::future::ready(());
        let code = link_gone(
            &mut streams,
            Some("amp2"),
            &RequestError::Closed,
            &mut interrupt,
            |streams, err| streams.fail(ExitCode::Failure, &err.to_string()),
        )
        .await;
        // Read the sink through the writer still open: only what this verb
        // flushed has reached it.
        (
            code,
            String::from_utf8(buffered.get_ref().to_vec()).unwrap(),
        )
    };

    assert_eq!(code, ExitCode::Success);
    assert_eq!(
        reached, "web | backlog\n",
        "an interrupt the operator meant ends the verb, not what it printed"
    );
    assert_eq!(String::from_utf8(err).unwrap(), "");
}
