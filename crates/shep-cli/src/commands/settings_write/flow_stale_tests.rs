//! The holder is read again after the dog is asked, because the ask waits.
//!
//! [`crate::dog_settings::ask_dog`] can hold a write for the dog's whole
//! `action_timeout`. A dashboard that files its own write on another key of the
//! same holder during that wait is the same edit the confirmation window already
//! guards against, and what this run sends is the whole holder: the plan built
//! before the ask carries every key it read.

use std::sync::{Arc, Mutex};

use shep_client::testing::{control_address, fake_client_answering_refusals};
use shep_core::protocol::{ActionOutcome, ActionReply, Request, Response};

use super::Flags;
use super::Verb;
use super::flow::run;
use super::unset::plan_section;
use crate::cli::Format;
use crate::dog_settings::validate_tests::{home_with, validating};
use crate::exit::ExitCode;
use crate::output::Streams;

/// The section text the shepherd answered, which is what the plan was built
/// from.
const ANSWERED: &str = "repo = \"/srv/project\"\n\n[worker]\ntimeout = 30\n";

/// The same section after the lookout pane filed a write of its own on a
/// different key of it, while the dog was being asked.
const MOVED: &str = "repo = \"/srv/project\"\n\n[worker]\ntimeout = 30\nkeep = 4\n";

/// `kelpie`'s own section, with `timeout` removed from it.
fn plan() -> super::Plan {
    let doc = ANSWERED.parse::<toml_edit::DocumentMut>().unwrap();
    plan_section(
        doc,
        "kelpie",
        &["worker".to_string(), "timeout".to_string()],
    )
    .unwrap()
}

/// A shepherd that answers the first read of the section with what the plan was
/// built from, accepts the ask, and answers the next read with a section that
/// moved in between.
async fn a_shepherd_that_moves_the_section_mid_ask(
    dir: &std::path::Path,
) -> (
    shep_client::Client,
    tokio::sync::mpsc::UnboundedReceiver<shep_core::protocol::Envelope>,
) {
    let reads = Arc::new(Mutex::new(0usize));
    fake_client_answering_refusals(&control_address(dir), move |request| match request {
        Request::DogConfig { .. } => {
            let mut reads = reads.lock().unwrap();
            *reads += 1;
            let toml = if *reads == 1 { ANSWERED } else { MOVED };
            Ok(Response::DogSection {
                toml: toml.to_string().into(),
            })
        }
        Request::Trigger { .. } => Ok(Response::Triggered(vec![ActionReply {
            id: 7,
            name: "kelpie".to_string(),
            outcome: ActionOutcome::Replied {
                body: r#"{"accept":true}"#.to_string(),
            },
        }])),
        _other => Ok(Response::Pong),
    })
    .await
}

/// Every request the run put on the wire.
fn sent(
    mut envelopes: tokio::sync::mpsc::UnboundedReceiver<shep_core::protocol::Envelope>,
) -> Vec<Request> {
    let mut out = Vec::new();
    while let Ok(envelope) = envelopes.try_recv() {
        out.push(envelope.body);
    }
    out
}

/// The dog's yes is about the table, not about the holder still being what it
/// was. A section that moved while the dog thought is refused as the stale plan
/// it is, and nothing is written.
#[tokio::test]
async fn a_section_that_moved_while_the_dog_was_asked_is_not_written() {
    let dir = tempfile::tempdir().unwrap();
    let paths = home_with(dir.path(), Some(validating()));
    let (client, envelopes) = a_shepherd_that_moves_the_section_mid_ask(dir.path()).await;
    let plan = plan();

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        run(
            &client,
            &mut streams,
            &paths,
            "kelpie",
            "worker.timeout",
            &plan,
            Flags {
                verb: Verb::Unset,
                yes: true,
                dry_run: false,
            },
        )
        .await
    };

    let said = String::from_utf8_lossy(&err).to_string();
    assert_eq!(code, ExitCode::Failure, "{said}");
    assert!(
        said.contains("`kelpie`'s section changed while `shep unset` waited for the answer"),
        "the stale-plan refusal, in the words that say when it moved: {said}"
    );

    let requests = sent(envelopes);
    let asked = requests
        .iter()
        .filter(|request| matches!(request, Request::Trigger { .. }))
        .count();
    let read = requests
        .iter()
        .filter(|request| matches!(request, Request::DogConfig { .. }))
        .count();
    assert_eq!(asked, 1, "the dog was asked: {requests:?}");
    assert_eq!(
        read, 2,
        "the holder is read once before the ask and once after it: {requests:?}"
    );
    assert!(
        !requests
            .iter()
            .any(|request| matches!(request, Request::SetDogConfig { .. })),
        "a plan this stale writes nothing: {requests:?}"
    );
}

/// The same run, with a holder that stayed where it was: the second read costs
/// a request and refuses nothing.
#[tokio::test]
async fn a_section_that_held_still_writes_after_the_dog_answers() {
    let dir = tempfile::tempdir().unwrap();
    let paths = home_with(dir.path(), Some(validating()));
    let (client, envelopes) =
        fake_client_answering_refusals(&control_address(dir.path()), |request| match request {
            Request::DogConfig { .. } => Ok(Response::DogSection {
                toml: ANSWERED.to_string().into(),
            }),
            Request::Trigger { .. } => Ok(Response::Triggered(vec![ActionReply {
                id: 7,
                name: "kelpie".to_string(),
                outcome: ActionOutcome::Replied {
                    body: r#"{"accept":true}"#.to_string(),
                },
            }])),
            Request::SetDogConfig { name, .. } => Ok(Response::DogConfigSet { name: name.clone() }),
            _other => Ok(Response::Pong),
        })
        .await;
    let plan = plan();

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        run(
            &client,
            &mut streams,
            &paths,
            "kelpie",
            "worker.timeout",
            &plan,
            Flags {
                verb: Verb::Unset,
                yes: true,
                dry_run: false,
            },
        )
        .await
    };

    let said = String::from_utf8_lossy(&err).to_string();
    assert_eq!(code, ExitCode::Success, "{said}");
    let requests = sent(envelopes);
    assert!(
        requests
            .iter()
            .any(|request| matches!(request, Request::SetDogConfig { .. })),
        "the write went out: {requests:?}"
    );
}
