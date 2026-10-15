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
