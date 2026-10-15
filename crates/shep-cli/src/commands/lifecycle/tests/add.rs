use super::*;

/// `add` and `start` are one code path, so this guards a mode that
/// leaked rather than a missing feature.
#[tokio::test]
async fn add_registers_a_fresh_app_and_sends_no_start() {
    let dir = tempfile::tempdir().unwrap();
    let flockfile = dir.path().join("Flockfile.toml");
    std::fs::write(
        &flockfile,
        "[[app]]\nname = \"demo\"\nscript = \"/bin/sleep\"\n",
    )
    .unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&path, a_daemon_that_registers(Vec::new())).await;

    let (code, _out, _err) = add_against(&client, flockfile.to_str().unwrap()).await;

    assert_eq!(code, ExitCode::Success);
    let sent = sent(&mut envelopes);
    let registered: Vec<&str> = sent
        .iter()
        .filter_map(|request| match request {
            Request::Add { apps } => Some(apps[0].name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(registered, vec!["demo"], "one registration, for the app");
    assert!(
        !sent.iter().any(|r| matches!(r, Request::Start { .. })),
        "nothing was started"
    );
}

/// `add` exists so an operator can fill in the empty `env` keys a
/// template shipped, and a key nothing established is one the next load
/// overwrites.
#[tokio::test]
async fn add_establishes_the_keys_the_template_declared() {
    let dir = tempfile::tempdir().unwrap();
    let flockfile = dir.path().join("Flockfile.toml");
    std::fs::write(
        &flockfile,
        "[[app]]\nname = \"demo\"\nscript = \"/bin/sleep\"\n",
    )
    .unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&path, a_daemon_that_registers(Vec::new())).await;

    let (code, _out, _err) = add_against(&client, flockfile.to_str().unwrap()).await;

    assert_eq!(code, ExitCode::Success);
    let sent = sent(&mut envelopes);
    let add_at = sent
        .iter()
        .position(|r| matches!(r, Request::Add { .. }))
        .expect("the app was registered");
    let apply_at = sent
        .iter()
        .position(|r| matches!(r, Request::ApplyConfig { .. }))
        .expect("its declared keys were established");
    assert!(
        add_at < apply_at,
        "the app is registered before its keys are established"
    );
}

/// Re-running a template after editing it is ordinary, so the file's new
/// keys merge in and the running child survives it.
#[tokio::test]
async fn add_merges_into_a_running_app_without_replacing_it() {
    let dir = tempfile::tempdir().unwrap();
    let flockfile = dir.path().join("Flockfile.toml");
    std::fs::write(
        &flockfile,
        "[[app]]\nname = \"zam\"\nscript = \"/bin/sleep\"\n",
    )
    .unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&path, a_daemon_that_registers(a_clustered_flock(&[0]))).await;

    let (code, _out, _err) = add_against(&client, flockfile.to_str().unwrap()).await;

    assert_eq!(code, ExitCode::Success);
    let sent = sent(&mut envelopes);
    assert!(
        sent.iter()
            .any(|r| matches!(r, Request::ApplyConfig { .. })),
        "the template was merged in"
    );
    assert!(
        !sent.iter().any(|r| matches!(r, Request::Restart { .. })),
        "and nothing was replaced"
    );
    assert!(
        !sent.iter().any(|r| matches!(r, Request::Add { .. })),
        "the flock already has it, so there was nothing to register"
    );
}

/// `add` registers and stops, so a `NeedsRespawn` field it writes onto a
/// sheep the flock already has parks with nothing to promote it. The write is
/// `add`'s, and so is saying the value is not in force yet.
#[tokio::test]
async fn add_reports_the_stamp_it_left_waiting_on_a_spawn() {
    use shep_core::status::ProcStatus;

    let dir = tempfile::tempdir().unwrap();
    let flockfile = dir.path().join("Flockfile.toml");
    std::fs::write(
        &flockfile,
        "[[app]]\nname = \"zam\"\nscript = \"/bin/sleep\"\n",
    )
    .unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let flock = vec![ProcessInfo::builder(0, "zam", ProcStatus::Stopped).build()];
    // A Flockfile target declares `script`, so this load sends `ApplyConfig`
    // before the flag write, and `apply_declared` treats any other answer as
    // a daemon-side fault.
    let (client, mut envelopes) =
        fake_client_answering(&path, move |request: &Request| match request {
            Request::ListFlock => Response::Flock(flock.clone()),
            Request::ApplyConfig { apps, .. } => Response::Applied(
                apps.iter()
                    .map(|app| {
                        SheepApplied::new(app.config.name.clone(), Vec::new(), Vec::new(), None)
                    })
                    .collect(),
            ),
            Request::SetSheepField { name, key, .. } => Response::SheepFieldSet {
                name: name.clone(),
                key: key.clone(),
                pending: true,
                warning: None,
            },
            _ => Response::Pong,
        })
        .await;

    let mut args = start_args(flockfile.to_str().unwrap());
    args.no_log_timestamps = true;
    let (code, _out, said) = add_against_with_args(&client, &args).await;

    assert_eq!(code, ExitCode::Success, "{said}");
    let bodies = sent(&mut envelopes);
    assert!(
        bodies
            .iter()
            .any(|r| matches!(r, Request::ApplyConfig { .. })),
        "the Flockfile's own keys merged first, which is what the flag write \
         cannot ride on: {bodies:?}"
    );
    let written: Vec<String> = bodies
        .into_iter()
        .filter_map(|request| match request {
            Request::SetSheepField { name, key, value } => {
                assert_eq!(name, "zam", "the sheep the flock already has");
                assert_eq!(value, serde_json::json!(false));
                Some(key)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        written,
        vec!["log_timestamps".to_string()],
        "add writes the flag onto a sheep the flock already has, the way start \
         does: {said}"
    );
    assert!(
        said.contains(
            "notice[add]: zam: log_timestamps waiting on the next spawn \
             (`shep reload zam` promotes them)"
        ),
        "and nothing in this verb spawns, so it says the value is parked: {said}"
    );
}

/// The same write for the target `add` reaches without reading a file: the
/// sheep's own name. Nothing under `add` spawns, so the parked clause is the
/// whole report.
#[tokio::test]
async fn add_reports_the_stamp_it_left_waiting_on_a_sheep_the_token_named() {
    use shep_client::testing::fake_client_answering;
    use shep_core::status::ProcStatus;

    let dir = tempfile::tempdir().unwrap();
    let sock = shep_client::testing::control_address(dir.path());
    let flock = vec![ProcessInfo::builder(0, "zam", ProcStatus::Stopped).build()];
    let (client, mut envelopes) =
        fake_client_answering(&sock, move |request: &Request| match request {
            Request::ListFlock => Response::Flock(flock.clone()),
            Request::SetSheepField { name, key, .. } => Response::SheepFieldSet {
                name: name.clone(),
                key: key.clone(),
                pending: true,
                warning: None,
            },
            _ => Response::Pong,
        })
        .await;

    let mut args = start_args("zam");
    args.no_log_timestamps = true;
    let (code, _out, said) = add_against_with_args(&client, &args).await;

    assert_eq!(code, ExitCode::Success, "{said}");
    let written: Vec<String> = sent(&mut envelopes)
        .into_iter()
        .filter_map(|request| match request {
            Request::SetSheepField { name, key, value } => {
                assert_eq!(name, "zam", "the token named this sheep");
                assert_eq!(value, serde_json::json!(false));
                Some(key)
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        written,
        vec!["log_timestamps".to_string()],
        "a named target takes the same early return a Flockfile target does \
         not, and the flag cannot be dropped by it: {said}"
    );
    assert!(
        said.contains(
            "notice[add]: zam: log_timestamps waiting on the next spawn \
             (`shep reload zam` promotes them)"
        ),
        "and it is said parked: {said}"
    );
}

/// A name target reads no Flockfile, and `add` sits behind that same
/// boundary. Nothing is left for the verb to do but say so.
#[tokio::test]
async fn add_by_name_registers_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&path, a_daemon_that_registers(a_clustered_flock(&[0]))).await;

    let (code, _out, said) = add_against(&client, "zam").await;

    assert_eq!(code, ExitCode::Success);
    let sent = sent(&mut envelopes);
    assert!(
        !sent.iter().any(|r| matches!(r, Request::Add { .. })),
        "the flock already has it"
    );
    assert!(
        !sent
            .iter()
            .any(|r| matches!(r, Request::ApplyConfig { .. })),
        "a name target reads no file, so there is nothing to apply"
    );
    assert!(
        said.contains("already registered"),
        "the operator is told why nothing happened: {said}"
    );
}
