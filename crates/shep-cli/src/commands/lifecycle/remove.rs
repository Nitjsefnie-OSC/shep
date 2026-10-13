//! `shep delete`: the one way to remove a sheep or a dog, and it sticks.
//!
//! The verb takes selectors, as it always has. What it now asks depends on
//! what each selector names in this home's `shep.toml`: a dog goes through
//! [`crate::commands::dogs::remove_dog`], which strikes the adoption as well
//! as asking the shepherd to deregister it, so the next boot does not bring it
//! back. Everything else goes through the shepherd, unchanged.
//!
//! A built-in dog's name is deliberately left out of the dog half. It is in
//! neither `enabled_dogs` nor `adopted_dogs` unless an operator wrote it
//! there, and the question "is this name a dog or a sheep that kept the name"
//! has one honest answerer: the shepherd that is holding the entry. So the
//! name travels the sheep path and the shepherd either refuses it or deletes
//! the sheep, whichever it is.
//!
//! A `shep.toml` that exists and cannot be read or parsed answers that same
//! question for no name, so a name selector is refused there rather than sent
//! off to be guessed at. The shepherd would delete the live entry, the
//! adoption would survive in the file the next boot reads, and a verb that
//! promises "for good" would exit 0 saying `deleted 1 sheep`. A sweep needs no
//! answer and is untouched.

use shep_core::paths::ShepPaths;
use shep_core::protocol::SelectorSpec;

use crate::cli::SelectorArgs;
use crate::commands::dogs::{self, Removal, fail_config};
use crate::commands::lifecycle::operational::delete_sheep;
use crate::commands::lifecycle::selectors::parse_selectors;
use crate::commands::shep_toml::{ShepToml, ShepTomlError};
use crate::exit::ExitCode;
use crate::output::Streams;
use crate::version_guard::{VersionGuard, connect_client};

/// `shep delete <selectors...>`
///
/// Dogs first, then sheep: a dog's removal is the one that edits `shep.toml`,
/// and a sheep's failure says nothing about whether that edit landed.
///
/// Each half renders its own outcome, so `--format json` on a command that
/// names both gets one envelope per half. A mixed `delete` is rare enough that
/// the alternative, one payload holding a dog's row beside a sheep's ids, was
/// not worth a row type neither verb asked for.
///
/// # Errors
/// [`ExitCode::InvalidConfig`] or [`ExitCode::Failure`] after writing what
/// went wrong, when this home's `shep.toml` exists and cannot be read or
/// parsed and a selector is an exact name. Nothing is sent and nothing is
/// deleted on that arm.
pub(crate) async fn delete(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    guard: VersionGuard,
    args: &SelectorArgs,
) -> ExitCode {
    let selectors = match parse_selectors(streams, &args.selectors) {
        Ok(selectors) => selectors,
        Err(code) => return code,
    };
    let mut dog_names: Vec<String> = Vec::new();
    let mut sheep_tokens: Vec<String> = Vec::new();
    for (raw, selector) in args.selectors.iter().zip(&selectors) {
        match dog_in_config(paths, selector) {
            Ok(Some(name)) => dog_names.push(name),
            Ok(None) => sheep_tokens.push(raw.clone()),
            Err(err) => return fail_config(streams, &err),
        }
    }

    let mut failure: Option<ExitCode> = None;
    for name in &dog_names {
        let code = dogs::remove_dog(streams, paths, name, Removal::Delete).await;
        if code != ExitCode::Success {
            failure = failure.or(Some(code));
        }
    }
    if sheep_tokens.is_empty() {
        return failure.unwrap_or(ExitCode::Success);
    }
    let client = match connect_client(streams, paths, guard).await {
        Ok(client) => client,
        Err(code) => return code,
    };
    let sheep = SelectorArgs {
        selectors: sheep_tokens,
    };
    match delete_sheep(&client, streams, &sheep).await {
        ExitCode::Success => failure.unwrap_or(ExitCode::Success),
        code => failure.unwrap_or(code),
    }
}

/// The dog `selector` names, or `None` when this verb should send it to the
/// shepherd as a sheep
///
/// Only an exact name reaches a dog: `all`, a fold and a regex are the flock,
/// and the shepherd already passes dogs by for an acting verb. The name is
/// read off `shep.toml`, the one place this CLI knows what it adopted, and a
/// built-in name is passed by for the reason this module's header gives.
///
/// # Errors
/// [`ShepTomlError::Io`] or [`ShepTomlError::Parse`] when `shep.toml` exists
/// and this cannot read it. A missing file is an empty document, which answers
/// `None` for every name, and a sweep is `Ok(None)` without reading at all.
fn dog_in_config(
    paths: &ShepPaths,
    selector: &SelectorSpec,
) -> Result<Option<String>, ShepTomlError> {
    let SelectorSpec::Name(name) = selector else {
        return Ok(None);
    };
    if crate::dog::BUILT_IN_DOGS.contains(&name.as_str()) {
        return Ok(None);
    }
    let cfg = ShepToml::read_only(&paths.daemon_config)?;
    let known = cfg
        .adopted_dog_names()
        .iter()
        .chain(cfg.enabled_dog_names().iter())
        .any(|dog| dog == name);
    Ok(known.then(|| name.clone()))
}

#[cfg(all(test, unix))]
mod tests {
    use std::path::Path;

    use shep_core::protocol::{Envelope, HelloAck, PROTOCOL_VERSION, Request, Response};

    use super::*;
    use crate::cli::Format;

    fn args(selectors: &[&str]) -> SelectorArgs {
        SelectorArgs {
            selectors: selectors.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// A `$SHEP_HOME` holding one adopted dog named `name` with a section of
    /// its own, with its `run` directory there so a fake shepherd can bind
    /// `paths.socket`
    fn home_with_a_dog(dir: &Path, name: &str) -> ShepPaths {
        let paths = ShepPaths::resolve(&|_| None, dir);
        std::fs::create_dir_all(&paths.run).unwrap();
        ShepToml::edit(&paths.daemon_config, |cfg| {
            cfg.adopt_dog(name, Path::new("/usr/local/bin/shep-otel"), false)
                .unwrap();
        })
        .unwrap();
        std::fs::write(&paths.dogs_config, "[log-rotate]\ndebounce = \"30s\"\n").unwrap();
        paths
    }

    /// A shepherd that answers every `Delete` with one id, announcing this
    /// build's own version so the version guard lets the verb through
    async fn fake_shepherd(paths: &ShepPaths) -> tokio::sync::mpsc::UnboundedReceiver<Envelope> {
        shep_client::testing::fake_daemon_answering_with_ack(
            &paths.socket,
            HelloAck {
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol: PROTOCOL_VERSION,
                pid: 4242,
                min_supported: None,
            },
            |request| match request {
                Request::Delete { .. } => Response::Deleted(vec![7]),
                Request::ListFlock => Response::Flock(Vec::new()),
                _ => Response::Pong,
            },
        )
        .await
    }

    /// Runs `delete` against `paths`, handing back its code, stdout and stderr
    ///
    /// `envelopes` is the fake's capture, drained afterwards: `None` stands
    /// for no shepherd running, which is the one connect failure this verb's
    /// dog half tolerates.
    async fn delete_against(
        paths: &ShepPaths,
        mut envelopes: Option<&mut tokio::sync::mpsc::UnboundedReceiver<Envelope>>,
        selectors: &[&str],
        fmt: Format,
    ) -> (ExitCode, String, String, Vec<Request>) {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt,
        };
        let code = delete(&mut streams, paths, VersionGuard::Enforce, &args(selectors)).await;
        let mut sent = Vec::new();
        if let Some(rx) = envelopes.as_mut() {
            while let Ok(envelope) = rx.try_recv() {
                sent.push(envelope.body);
            }
        }
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
            sent,
        )
    }

    /// The whole point of the ticket: `delete` is the verb that removes a dog,
    /// and it removes it from the config the next boot reads as well as from
    /// the flock.
    #[tokio::test]
    async fn deleting_an_adopted_dog_forgets_the_adoption_and_asks_the_shepherd() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, out, err, sent) =
            delete_against(&paths, Some(&mut envelopes), &["log-rotate"], Format::Table).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert_eq!(
            sent,
            vec![Request::Delete {
                selector: SelectorSpec::Name("log-rotate".to_string()),
            }],
            "the dog half still travels the shepherd's own delete"
        );
        let written = std::fs::read_to_string(&paths.daemon_config).unwrap();
        assert!(
            !written.contains("log-rotate"),
            "a dog left in enabled_dogs comes back at the next boot: {written}"
        );
        assert!(
            err.contains("kept [log-rotate] in dogs.toml"),
            "the settings that survive a dog's removal are named: {err}"
        );
        let kept = std::fs::read_to_string(&paths.dogs_config).unwrap();
        assert!(
            kept.contains("[log-rotate]"),
            "the section an operator wrote is theirs, not the adoption's: {kept}"
        );
        assert!(
            !out.contains("deleted 1 sheep"),
            "a dog is not a sheep, and its ids are not what this verb printed: {out}"
        );
    }

    /// `--format json` answers with `delete`'s own reply, not the hidden
    /// verb's: the `command` key is what a script already filtering on
    /// `delete` reads.
    #[tokio::test]
    async fn deleting_a_dog_answers_under_delete() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, out, err, _) =
            delete_against(&paths, Some(&mut envelopes), &["log-rotate"], Format::Json).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        let json: serde_json::Value = serde_json::from_slice(out.as_bytes()).unwrap();
        assert_eq!(json["command"], "delete", "{json}");
        assert_eq!(json["data"]["name"], "log-rotate", "{json}");
        assert_eq!(json["data"]["status"], "stopped", "{json}");
    }

    /// A sheep's name reaches the shepherd exactly as it did before this verb
    /// learned about dogs: same request, same reply, same stderr line, and no
    /// dog config spent on the way.
    #[tokio::test]
    async fn deleting_a_sheep_is_untouched_by_the_dog_half() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, _, err, sent) =
            delete_against(&paths, Some(&mut envelopes), &["web"], Format::Table).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert_eq!(
            sent,
            vec![
                Request::Delete {
                    selector: SelectorSpec::Name("web".to_string()),
                },
                Request::ListFlock,
            ],
            "{sent:?}"
        );
        assert!(
            err.contains("deleted 1 sheep, id 7"),
            "the sheep half keeps its own stderr line: {err}"
        );
        let written = std::fs::read_to_string(&paths.daemon_config).unwrap();
        assert!(
            written.contains("log-rotate"),
            "deleting a sheep spends no dog config: {written}"
        );
    }

    /// A built-in dog's name is not this CLI's call to make. It goes out as a
    /// selector and comes back as whatever the shepherd holding the entry
    /// says, which is a refusal for a dog and a delete for a sheep that kept
    /// the name.
    #[tokio::test]
    async fn a_built_in_dogs_name_travels_the_sheep_path() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        ShepToml::try_edit(&paths.daemon_config, |cfg| cfg.enable_dog("metrics")).unwrap();
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, _, _, sent) =
            delete_against(&paths, Some(&mut envelopes), &["metrics"], Format::Json).await;

        assert_eq!(code, ExitCode::Success);
        assert_eq!(
            sent,
            vec![Request::Delete {
                selector: SelectorSpec::Name("metrics".to_string()),
            }],
            "the shepherd is the one that knows whether this name is a dog"
        );
        let written = std::fs::read_to_string(&paths.daemon_config).unwrap();
        assert!(
            written.contains("metrics"),
            "a name this CLI could not judge spends no config edit: {written}"
        );
    }

    /// A sweep is the flock, and a dog is not in it: `all` must not be the
    /// command that quietly forgets an adoption.
    #[tokio::test]
    async fn a_wildcard_never_takes_the_dog_half() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, _, _, sent) =
            delete_against(&paths, Some(&mut envelopes), &["all"], Format::Json).await;

        assert_eq!(code, ExitCode::Success);
        assert_eq!(
            sent,
            vec![Request::Delete {
                selector: SelectorSpec::All,
            }],
            "{sent:?}"
        );
        let written = std::fs::read_to_string(&paths.daemon_config).unwrap();
        assert!(
            written.contains("log-rotate"),
            "`shep delete all` left the adoption exactly where it was: {written}"
        );
    }

    /// The two halves each answer, so a command naming both does both, and the
    /// dog half's config edit is not held hostage to the sheep half's round
    /// trip.
    #[tokio::test]
    async fn a_command_naming_a_dog_and_a_sheep_does_both() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, out, _, sent) = delete_against(
            &paths,
            Some(&mut envelopes),
            &["log-rotate", "web"],
            Format::Json,
        )
        .await;

        assert_eq!(code, ExitCode::Success);
        assert_eq!(
            sent,
            vec![
                Request::Delete {
                    selector: SelectorSpec::Name("log-rotate".to_string()),
                },
                Request::Delete {
                    selector: SelectorSpec::Name("web".to_string()),
                },
            ],
            "{sent:?}"
        );
        let json: Vec<serde_json::Value> = serde_json::Deserializer::from_str(&out)
            .into_iter::<serde_json::Value>()
            .map(|value| value.unwrap())
            .collect();
        assert_eq!(json.len(), 2, "{out}");
        assert_eq!(json[0]["command"], "delete", "{json:?}");
        assert_eq!(json[0]["data"]["name"], "log-rotate", "{json:?}");
        assert_eq!(json[1]["data"], serde_json::json!([7]), "{json:?}");
    }

    /// With no shepherd to deregister the entry, the config half still runs,
    /// which is the half that decides whether the dog comes back.
    #[tokio::test]
    async fn deleting_a_dog_with_no_shepherd_still_forgets_the_adoption() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");

        let (code, out, err, _) = delete_against(&paths, None, &["log-rotate"], Format::Json).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        let json: serde_json::Value = serde_json::from_slice(out.as_bytes()).unwrap();
        assert_eq!(json["command"], "delete", "{json}");
        assert_eq!(
            json["data"]["shepherd_acted"],
            serde_json::json!(false),
            "{json}"
        );
        let written = std::fs::read_to_string(&paths.daemon_config).unwrap();
        assert!(
            !written.contains("log-rotate"),
            "the next boot reads this file: {written}"
        );
    }

    /// A `shep.toml` this CLI cannot read answers "is this name a dog?" for no
    /// name, which is not a licence to guess. Sent off, the shepherd deletes
    /// the live entry, the adoption survives in the file the next boot reads,
    /// and a verb that promises "for good" exits 0 saying `deleted 1 sheep`.
    #[tokio::test]
    async fn a_shep_toml_that_cannot_be_parsed_refuses_a_name_and_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        let broken = "enabled_dogs = [\"log-rotate\"\n";
        std::fs::write(&paths.daemon_config, broken).unwrap();
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, out, err, sent) =
            delete_against(&paths, Some(&mut envelopes), &["log-rotate"], Format::Table).await;

        assert_eq!(code, ExitCode::InvalidConfig, "{err}");
        assert!(err.contains("shep.toml"), "{err}");
        assert!(out.is_empty(), "a refusal prints no deletion: {out}");
        assert!(sent.is_empty(), "nothing reached the shepherd: {sent:?}");
        let written = std::fs::read_to_string(&paths.daemon_config).unwrap();
        assert_eq!(written, broken, "a refusal spends no config edit");
    }

    /// The half the refusal rests on, held with no socket and no shepherd: an
    /// unreadable `shep.toml` answers the dog question with the failure, not
    /// with `None`, and a missing file is still "no dog here".
    #[test]
    fn dog_in_config_reports_an_unreadable_config_instead_of_answering_none() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ShepPaths::resolve(&|_| None, dir.path());
        let name = SelectorSpec::Name("log-rotate".to_string());

        assert!(
            matches!(dog_in_config(&paths, &name), Ok(None)),
            "a home with no shep.toml has no adopted dog"
        );

        std::fs::create_dir_all(&paths.run).unwrap();
        std::fs::write(&paths.daemon_config, "enabled_dogs = [\"log-rotate\"\n").unwrap();
        assert!(
            matches!(
                dog_in_config(&paths, &name),
                Err(ShepTomlError::Parse { .. })
            ),
            "a file that exists and does not parse is a failure this verb reports"
        );

        assert!(
            matches!(dog_in_config(&paths, &SelectorSpec::All), Ok(None)),
            "a sweep is answered without opening the file at all"
        );
    }

    /// The refusal is scoped to the selector that needs the answer. `all` is
    /// the flock, a dog is not in it, and a broken `shep.toml` costs that
    /// command nothing it did not already pay.
    #[tokio::test]
    async fn a_sweep_still_deletes_the_flock_over_an_unreadable_shep_toml() {
        let dir = tempfile::tempdir().unwrap();
        let paths = home_with_a_dog(dir.path(), "log-rotate");
        std::fs::write(&paths.daemon_config, "not toml at all").unwrap();
        let mut envelopes = fake_shepherd(&paths).await;

        let (code, _, err, sent) =
            delete_against(&paths, Some(&mut envelopes), &["all"], Format::Json).await;

        assert_eq!(code, ExitCode::Success, "{err}");
        assert_eq!(
            sent,
            vec![Request::Delete {
                selector: SelectorSpec::All
            }],
            "{sent:?}"
        );
    }
}
