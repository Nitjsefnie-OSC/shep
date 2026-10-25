//! A load whose target names a sheep the flock already has: the arm that
//! reads no file, so a flag with no template to ride on has to travel as a
//! write of its own.

use super::*;

/// `shep start <name>` is the ordinary way to bring a sheep the flock
/// already has back up, and it takes an early return no Flockfile target
/// takes: the token resolved to a sheep, so no file is read and no `AppConfig`
/// is built for the flag to ride on.
#[tokio::test]
async fn no_log_timestamps_on_a_sheep_named_by_the_token_writes_the_field_first() {
    use shep_client::testing::fake_client_answering;
    use shep_core::status::ProcStatus;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let flock = vec![ProcessInfo::builder(0, "rotating", ProcStatus::Stopped).build()];
    let (client, mut envelopes) =
        fake_client_answering(&sock, move |request: &Request| match request {
            Request::ListFlock => Response::Flock(flock.clone()),
            Request::SetSheepField { name, key, .. } => Response::SheepFieldSet {
                name: name.clone(),
                key: key.clone(),
                pending: true,
                warning: None,
            },
            Request::Restart { .. } => Response::Restarted {
                accepted: vec![ProcessInfo::builder(0, "rotating", ProcStatus::Online).build()],
                refused: Vec::new(),
            },
            _ => Response::Pong,
        })
        .await;

    let mut args = start_args("rotating");
    args.no_log_timestamps = true;
    let (code, _printed, said) = start_against_with_args(&client, &args).await;

    assert_eq!(code, ExitCode::Success, "{said}");
    let mut order = Vec::new();
    while let Ok(envelope) = envelopes.try_recv() {
        match envelope.body {
            Request::SetSheepField { name, key, value } => {
                assert_eq!(name, "rotating", "the token named this sheep");
                assert_eq!(key, "log_timestamps");
                assert_eq!(value, serde_json::json!(false));
                order.push("field".to_string());
            }
            Request::Restart { .. } => order.push("restart".to_string()),
            _ => {}
        }
    }
    assert_eq!(
        order,
        ["field", "restart"],
        "a named sheep is resumed without a merge, so the write is the only \
         route the flag has, and it goes first: {said}"
    );
    assert!(
        said.contains(
            "notice[start]: rotating: log_timestamps waiting on the next spawn \
             (`shep reload rotating` promotes them)"
        ),
        "the write parked, and the verb says so: {said}"
    );
}

/// The same arm without the flag: a named sheep is resumed, and nothing is
/// written that the operator did not ask for.
#[tokio::test]
async fn a_sheep_named_by_the_token_without_the_flag_gets_no_field_write() {
    use shep_client::testing::fake_client_answering;
    use shep_core::status::ProcStatus;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let flock = vec![ProcessInfo::builder(0, "rotating", ProcStatus::Stopped).build()];
    let (client, mut envelopes) =
        fake_client_answering(&sock, move |request: &Request| match request {
            Request::ListFlock => Response::Flock(flock.clone()),
            Request::Restart { .. } => Response::Restarted {
                accepted: vec![ProcessInfo::builder(0, "rotating", ProcStatus::Online).build()],
                refused: Vec::new(),
            },
            _ => Response::Pong,
        })
        .await;

    let (code, _printed, said) = start_against(&client, "rotating").await;

    assert_eq!(code, ExitCode::Success, "{said}");
    assert!(
        !respawns(&mut envelopes).is_empty(),
        "the resume is what this verb was asked for: {said}"
    );
}

/// The refusal case for the named arm: the sheep stays down rather than
/// coming back up with its stamps on.
#[tokio::test]
async fn a_refused_field_write_on_a_named_sheep_stops_before_the_resume() {
    use shep_client::testing::fake_client_answering_refusals;
    use shep_core::protocol::RpcError;
    use shep_core::status::ProcStatus;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let flock = vec![ProcessInfo::builder(0, "rotating", ProcStatus::Stopped).build()];
    let (client, mut envelopes) =
        fake_client_answering_refusals(&sock, move |request: &Request| match request {
            Request::ListFlock => Ok(Response::Flock(flock.clone())),
            Request::SetSheepField { .. } => Err(RpcError {
                code: RpcErrorCode::InvalidConfig,
                message: "no config field named log_timestamps".to_string(),
                daemon_version: None,
            }),
            _ => Ok(Response::Pong),
        })
        .await;

    let mut args = start_args("rotating");
    args.no_log_timestamps = true;
    let (code, printed, said) = start_against_with_args(&client, &args).await;

    assert_eq!(code, ExitCode::InvalidConfig, "{said}");
    assert!(printed.is_empty(), "a failed verb leaves stdout empty");
    assert!(
        said.contains("rotating: log_timestamps"),
        "the failure names the sheep and the value: {said}"
    );
    assert!(
        respawns(&mut envelopes).is_empty(),
        "and the sheep is not brought back up stamped"
    );
}

/// Runs `start` over a host link named `amp2`, handing back code, stdout and
/// stderr.
async fn start_over_host_with_args(
    client: &Client,
    args: &StartArgs,
) -> (ExitCode, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams::bare(&mut out, &mut err, Format::Table);
        start_on_host(client, &mut streams, args, "amp2").await
    };
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

/// A name the host's flock has is resumed the way a local one is, and
/// whatever Flockfile sits in this directory is not read.
#[tokio::test]
async fn a_known_name_over_a_host_is_resumed_with_no_local_file_read() {
    use shep_client::testing::fake_client_answering;
    use shep_core::status::ProcStatus;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let flock = vec![ProcessInfo::builder(0, "api", ProcStatus::Stopped).build()];
    let (client, mut envelopes) =
        fake_client_answering(&sock, move |request: &Request| match request {
            Request::ListFlock => Response::Flock(flock.clone()),
            Request::Restart { .. } => Response::Restarted {
                accepted: vec![ProcessInfo::builder(0, "api", ProcStatus::Online).build()],
                refused: Vec::new(),
            },
            _ => Response::Pong,
        })
        .await;

    let (code, _printed, said) = start_over_host_with_args(&client, &start_args("api")).await;

    assert_eq!(code, ExitCode::Success, "{said}");
    assert_eq!(respawns(&mut envelopes).len(), 1, "{said}");
}

/// Counts the requests of any kind that reached the wire.
fn requests_seen(
    envelopes: &mut tokio::sync::mpsc::UnboundedReceiver<shep_core::protocol::Envelope>,
) -> usize {
    let mut seen = 0;
    while envelopes.try_recv().is_ok() {
        seen += 1;
    }
    seen
}

/// A path would be read on this machine, so it is refused with the command
/// that reads it on the host, and the host is not asked anything.
#[tokio::test]
async fn a_path_target_over_a_host_refuses_with_the_exact_ssh_command() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&sock, |_: &Request| Response::Flock(Vec::new())).await;

    let (code, printed, said) = start_over_host_with_args(&client, &start_args("./api")).await;

    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(printed.is_empty(), "a failed verb leaves stdout empty");
    assert!(
        said.contains(
            "`shep --host amp2 start ./api` would read ./api on this machine. \
             To start it there, run: ssh amp2 shep start ./api"
        ),
        "{said}"
    );
    assert_eq!(requests_seen(&mut envelopes), 0, "{said}");
}

/// `-` and `--flockfile` read this machine too, whatever the token looks like.
#[tokio::test]
async fn stdin_and_flockfile_targets_over_a_host_refuse_the_same_way() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&sock, |_: &Request| Response::Flock(Vec::new())).await;

    let (code, _printed, said) = start_over_host_with_args(&client, &start_args("-")).await;
    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(
        said.contains("`shep --host amp2 start -` would read stdin on this machine."),
        "{said}"
    );
    assert!(said.contains("ssh amp2 shep start -"), "{said}");

    let mut args = start_args("web");
    args.flockfile = true;
    let (code, _printed, said) = start_over_host_with_args(&client, &args).await;
    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(
        said.contains("`shep --host amp2 start --flockfile web` would read web on this machine."),
        "{said}"
    );
    assert!(
        said.contains("ssh amp2 shep start --flockfile web"),
        "{said}"
    );
    assert_eq!(requests_seen(&mut envelopes), 0, "{said}");
}

/// A name the host does not know is a miss on the host, with the ssh command
/// in case it was meant as a script.
#[tokio::test]
async fn an_unknown_name_over_a_host_is_not_found_and_hints_at_ssh() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&sock, |_: &Request| Response::Flock(Vec::new())).await;

    let (code, printed, said) = start_over_host_with_args(&client, &start_args("ghost")).await;

    assert_eq!(code, ExitCode::NotFound, "{said}");
    assert!(printed.is_empty());
    assert!(
        said.contains("ghost is not a sheep or a fold on amp2"),
        "{said}"
    );
    assert!(said.contains("ssh amp2 shep start ghost"), "{said}");
    assert!(
        respawns(&mut envelopes).is_empty(),
        "nothing is started for a name the host lacks"
    );
}

/// A bare `start` would discover a Flockfile in this directory, and would
/// boot a shepherd, so over a host it is refused before either.
#[cfg(unix)]
#[tokio::test]
async fn a_bare_start_over_a_host_refuses_before_reading_the_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let link = crate::remote::Link::Host {
        dest: "amp2",
        options: shep_client::SshOptions::default().program(dir.path().join("no-such-ssh")),
    };
    let mut args = start_args("unused");
    args.targets.clear();
    let mut out = Vec::new();
    let mut err = Vec::new();

    let code = {
        let mut streams = Streams::bare(&mut out, &mut err, Format::Table);
        crate::client::load_command(
            &mut streams,
            &link,
            crate::version_guard::VersionGuard::Enforce,
            &args,
            Load::Start,
        )
        .await
    };

    let said = String::from_utf8(err).unwrap();
    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(
        said.contains("`shep --host amp2 start` would read the Flockfile in this directory"),
        "{said}"
    );
    assert!(said.contains("ssh amp2 shep start"), "{said}");
    assert!(out.is_empty());
}

/// The refusal for a target read on this machine needs no host, so an
/// unreachable one is not what the operator is told.
#[cfg(unix)]
#[tokio::test]
async fn a_path_target_over_an_unreachable_host_is_refused_not_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let link = crate::remote::Link::Host {
        dest: "amp2",
        options: shep_client::SshOptions::default().program(dir.path().join("no-such-ssh")),
    };
    for (targets, flockfile) in [(&["./api"][..], false), (&["-"], false), (&["web"], true)] {
        let mut args = start_args("unused");
        args.targets = targets.iter().map(ToString::to_string).collect();
        args.flockfile = flockfile;
        let mut out = Vec::new();
        let mut err = Vec::new();

        let code = {
            let mut streams = Streams::bare(&mut out, &mut err, Format::Table);
            crate::client::load_command(
                &mut streams,
                &link,
                crate::version_guard::VersionGuard::Enforce,
                &args,
                Load::Start,
            )
            .await
        };

        let said = String::from_utf8(err).unwrap();
        assert_eq!(code, ExitCode::Usage, "{targets:?}: {said}");
        assert!(said.contains("ssh amp2 shep start"), "{targets:?}: {said}");
    }
}

/// The hint starts the same thing on the host: every flag and assignment the
/// operator typed rides along.
#[tokio::test]
async fn the_ssh_hint_carries_every_flag_and_assignment() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let (client, _envelopes) =
        fake_client_answering(&sock, |_: &Request| Response::Flock(Vec::new())).await;
    let mut args = start_args("FOO=1");
    args.targets.push("./api".to_string());
    args.name = Some("api2".to_string());
    args.fold = Some("web".to_string());
    args.no_log_timestamps = true;

    let (code, _printed, said) = start_over_host_with_args(&client, &args).await;

    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(
        said.contains("ssh amp2 shep start --name api2 --fold web --no-log-timestamps FOO=1 ./api"),
        "{said}"
    );
}

/// The hint is pasted into this machine's shell and then read by the host's,
/// so a token with a space is quoted for both.
#[tokio::test]
async fn the_ssh_hint_survives_two_shells_for_a_token_with_a_space() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let (client, _envelopes) =
        fake_client_answering(&sock, |_: &Request| Response::Flock(Vec::new())).await;

    let (code, _printed, said) =
        start_over_host_with_args(&client, &start_args("./my api.js")).await;

    assert_eq!(code, ExitCode::Usage, "{said}");
    assert!(
        said.contains("`shep --host amp2 start './my api.js'` would read ./my api.js"),
        "{said}"
    );
    assert!(
        said.contains(r"ssh amp2 'shep start '\''./my api.js'\'''"),
        "{said}"
    );
}

/// A listing that failed says nothing about the name, so the failure is what
/// the operator hears, not a miss.
#[tokio::test]
async fn a_failed_listing_over_a_host_is_not_a_not_found() {
    use shep_client::testing::fake_client_answering_refusals;
    use shep_core::protocol::RpcError;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let (client, _envelopes) =
        fake_client_answering_refusals(&sock, |_: &Request| -> Result<Response, RpcError> {
            Err(RpcError {
                code: RpcErrorCode::Internal,
                message: "the roll is unreadable".to_string(),
                daemon_version: None,
            })
        })
        .await;

    let (code, printed, said) = start_over_host_with_args(&client, &start_args("web")).await;

    assert_eq!(code, ExitCode::Internal, "{said}");
    assert!(printed.is_empty());
    assert!(said.contains("the roll is unreadable"), "{said}");
    assert!(!said.contains("is not a sheep or a fold"), "{said}");
}
