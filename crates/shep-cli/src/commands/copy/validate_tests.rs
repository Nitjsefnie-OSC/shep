//! `shep copy` puts a dog's whole table past the same ask `shep set` does.
//!
//! The copy replaces a dog's table on the target sheep, which is the write a
//! dog that declared `validate` asked to see. Without the ask here, a table the
//! dog refused on `shep set` would land on another sheep through a door the
//! operator never thinks of as an edit to that dog's settings.

use shep_client::testing::{control_address, fake_client_answering_refusals};
use shep_core::protocol::{ActionOutcome, ActionReply, Envelope, Request, Response, RpcError};

use super::Write;
use super::write_plan;
use crate::cli::Format;
use crate::dog_settings::validate_tests::{home_with, validating};
use crate::exit::ExitCode;
use crate::output::Streams;

/// The table the copy would leave on `api`.
fn table() -> shep_core::config::DogTable {
    serde_json::json!({"checkout": "/srv/project", "interval": 30})
        .as_object()
        .cloned()
        .unwrap()
        .into()
}

/// Every request the run put on the wire.
fn sent(mut envelopes: tokio::sync::mpsc::UnboundedReceiver<Envelope>) -> Vec<Request> {
    let mut out = Vec::new();
    while let Ok(envelope) = envelopes.try_recv() {
        out.push(envelope.body);
    }
    out
}

/// A shepherd that answers the ask with `outcome` and takes any write after it.
fn a_shepherd(
    outcome: ActionOutcome,
) -> impl Fn(&Request) -> Result<Response, RpcError> + Send + Sync + 'static {
    move |request| match request {
        Request::Trigger { .. } => Ok(Response::Triggered(vec![ActionReply {
            id: 7,
            name: "kelpie".to_string(),
            outcome: outcome.clone(),
        }])),
        Request::SetSheepDogSettings { .. } => Ok(Response::SheepDogSettingsSet {
            name: "api".to_string(),
            dog: "kelpie".to_string(),
        }),
        Request::SetSheepField { .. } => Ok(Response::SheepFieldSet {
            name: "api".to_string(),
            key: "max_memory".to_string(),
            pending: false,
            warning: None,
        }),
        _other => Ok(Response::Pong),
    }
}

/// Runs the copy's write half against `daemon`, and hands back the code, what
/// stderr says, and what went out.
async fn copied(
    dir: &std::path::Path,
    manifest: Option<&str>,
    daemon: impl Fn(&Request) -> Result<Response, RpcError> + Send + Sync + 'static,
    write: Write,
) -> (ExitCode, String, Vec<Request>) {
    let paths = home_with(dir, manifest);
    let (client, envelopes) = fake_client_answering_refusals(&control_address(dir), daemon).await;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let result = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        write_plan(&client, &mut streams, &paths, "api", write).await
    };
    let code = match result {
        Ok(pending) => {
            let _ = pending;
            ExitCode::Success
        }
        Err(code) => code,
    };
    (
        code,
        String::from_utf8_lossy(&err).to_string(),
        sent(envelopes),
    )
}

/// The copy carries a dog's whole table, so the dog that declared `validate`
/// sees it, and its refusal is what stops the copy.
#[tokio::test]
async fn a_dog_that_refuses_stops_the_copy_of_its_table() {
    let dir = tempfile::tempdir().unwrap();
    let (code, said, requests) = copied(
        dir.path(),
        Some(validating()),
        a_shepherd(ActionOutcome::Replied {
            body: r#"{"accept":false,"reason":"this checkout belongs to another project"}"#
                .to_string(),
        }),
        Write::DogTable {
            dog: "kelpie".to_string(),
            table: table(),
        },
    )
    .await;

    assert_eq!(code, ExitCode::InvalidConfig, "{said}");
    assert!(
        said.contains("`kelpie` refused `api`'s table for `kelpie`")
            && said.contains("this checkout belongs to another project"),
        "the dog's own sentence, naming the copy it refused: {said}"
    );
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, Request::SetSheepDogSettings { .. })),
        "a copy a dog refused writes nothing: {requests:?}"
    );
    let asked = requests
        .iter()
        .filter(|request| matches!(request, Request::Trigger { .. }))
        .count();
    assert_eq!(asked, 1, "the dog was asked about the table: {requests:?}");
}

/// The ask carries the table the target would hold afterwards, whole, which is
/// what the copy replaces rather than what it copied.
#[tokio::test]
async fn the_ask_carries_the_whole_table_the_copy_leaves() {
    let dir = tempfile::tempdir().unwrap();
    let (_code, _said, requests) = copied(
        dir.path(),
        Some(validating()),
        a_shepherd(ActionOutcome::Replied {
            body: r#"{"accept":true}"#.to_string(),
        }),
        Write::DogTable {
            dog: "kelpie".to_string(),
            table: table(),
        },
    )
    .await;

    let Request::Trigger { params, .. } = requests
        .iter()
        .find(|request| matches!(request, Request::Trigger { .. }))
        .expect("the dog was asked")
    else {
        unreachable!("the ask is a trigger");
    };
    let params: serde_json::Value = serde_json::from_str(params.as_deref().unwrap()).unwrap();
    assert_eq!(
        params["table"],
        serde_json::json!({"checkout": "/srv/project", "interval": 30}),
        "{params}"
    );
    assert_eq!(
        params["sheep"],
        serde_json::json!("api"),
        "a copy writes a sheep's table for the dog, so the sheep is named"
    );
}

/// A dog with no `validate` in what shep read for it is never asked, and the
/// copy goes out as it always did.
#[tokio::test]
async fn a_copy_of_a_table_for_a_dog_that_declared_nothing_goes_straight_through() {
    let dir = tempfile::tempdir().unwrap();
    let (code, said, requests) = copied(
        dir.path(),
        Some(r#"{"manifest":1,"name":"kelpie","version":"0.2.0","protocol":12,"channel":true}"#),
        a_shepherd(ActionOutcome::Replied {
            body: r#"{"accept":false,"reason":"should never be asked"}"#.to_string(),
        }),
        Write::DogTable {
            dog: "kelpie".to_string(),
            table: table(),
        },
    )
    .await;

    assert_eq!(code, ExitCode::Success, "{said}");
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, Request::Trigger { .. })),
        "nothing was asked: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| matches!(request, Request::SetSheepDogSettings { .. })),
        "the write went out: {requests:?}"
    );
}

/// A sheep's own field is no dog's settings, so a copy of one is not asked
/// about even when the flock has a dog that wants to be consulted.
#[tokio::test]
async fn a_copy_of_a_sheeps_own_field_asks_no_dog() {
    let dir = tempfile::tempdir().unwrap();
    let (code, said, requests) = copied(
        dir.path(),
        Some(validating()),
        a_shepherd(ActionOutcome::Replied {
            body: r#"{"accept":false,"reason":"should never be asked"}"#.to_string(),
        }),
        Write::Field {
            name: "max_memory".to_string(),
            value: serde_json::json!(1_048_576),
        },
    )
    .await;

    assert_eq!(code, ExitCode::Success, "{said}");
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, Request::Trigger { .. })),
        "no dog's table is in this write: {requests:?}"
    );
}
