//! Typed shorthands for the requests more than one caller makes.
//!
//! Each sends one [`Request`] and answers the one [`Response`] it expects,
//! so a caller never writes the match or the error for any other answer.
//! [`Client`] and [`ReconnectingClient`] carry the same set.

use core::future::Future;

use shep_core::protocol::{
    DogSectionToml, HostUsage, ProcessInfo, Request, Response, RpcError, RpcErrorCode, SelectorSpec,
};
use shep_core::secrets::references_in;

use crate::ReconnectingClient;
use crate::client::{Client, LOG_PLANE_DEADLINE, RequestError};

impl Client {
    /// The dog `name`'s own section of `dogs.toml` with every
    /// `{{secret:...}}` reference resolved, empty when it has none.
    ///
    /// Asks [`Request::DogConfigResolved`]. A shepherd too old to know it
    /// is asked [`Request::DogConfig`] instead, and a section that still
    /// holds a reference is refused rather than handed over unresolved.
    ///
    /// # Errors
    ///
    /// As [`Self::request`], and [`RequestError::UnexpectedReply`] for any
    /// answer but [`Response::DogSection`]. [`RequestError::Rpc`] with
    /// [`RpcErrorCode::Unsupported`] when an older shepherd's section names
    /// a secret reference it cannot resolve.
    pub async fn dog_config(&self, name: &str) -> Result<DogSectionToml, RequestError> {
        resolved_dog_section(name, |body| self.request(body)).await
    }

    /// Every supervised entry, dogs included.
    ///
    /// # Errors
    ///
    /// As [`Self::request`], and [`RequestError::UnexpectedReply`] for any
    /// answer but [`Response::Flock`].
    pub async fn list_flock(&self) -> Result<Vec<ProcessInfo>, RequestError> {
        flock(self.request(Request::ListFlock).await?)
    }

    /// Reopens the log files of every sheep `selector` matches, under
    /// [`LOG_PLANE_DEADLINE`], answering each one it matched.
    ///
    /// # Errors
    ///
    /// As [`Self::request`], and [`RequestError::UnexpectedReply`] for any
    /// answer but [`Response::Reopened`].
    pub async fn reopen(&self, selector: SelectorSpec) -> Result<Vec<ProcessInfo>, RequestError> {
        let body = Request::Reopen { selector };
        reopened(
            self.request_with_deadline(body, Some(LOG_PLANE_DEADLINE))
                .await?,
        )
    }

    /// What the flock's machine is doing, or `None` where the shepherd
    /// cannot read its host at all.
    ///
    /// # Errors
    ///
    /// As [`Self::request`], and [`RequestError::UnexpectedReply`] for any
    /// answer but [`Response::HostUsage`].
    pub async fn host_usage(&self) -> Result<Option<HostUsage>, RequestError> {
        host(self.request(Request::HostUsage).await?)
    }
}

impl ReconnectingClient {
    /// Shorthand for [`Client::dog_config`] on the current generation.
    ///
    /// # Errors
    ///
    /// As [`Client::dog_config`].
    pub async fn dog_config(&self, name: &str) -> Result<DogSectionToml, RequestError> {
        resolved_dog_section(name, |body| self.request(body)).await
    }

    /// Shorthand for [`Client::list_flock`] on the current generation.
    ///
    /// # Errors
    ///
    /// As [`Client::list_flock`].
    pub async fn list_flock(&self) -> Result<Vec<ProcessInfo>, RequestError> {
        flock(self.request(Request::ListFlock).await?)
    }

    /// Shorthand for [`Client::reopen`] on the current generation.
    ///
    /// # Errors
    ///
    /// As [`Client::reopen`].
    pub async fn reopen(&self, selector: SelectorSpec) -> Result<Vec<ProcessInfo>, RequestError> {
        let body = Request::Reopen { selector };
        reopened(
            self.request_with_deadline(body, Some(LOG_PLANE_DEADLINE))
                .await?,
        )
    }

    /// Shorthand for [`Client::host_usage`] on the current generation.
    ///
    /// # Errors
    ///
    /// As [`Client::host_usage`].
    pub async fn host_usage(&self) -> Result<Option<HostUsage>, RequestError> {
        host(self.request(Request::HostUsage).await?)
    }
}

fn dog_config_request(name: &str) -> Request {
    Request::DogConfigResolved {
        name: name.to_owned(),
    }
}

/// Asks `ask` for the resolved section, and for the plain one when the
/// shepherd does not know the request.
///
/// A shepherd built before the request refuses it with `Unsupported`, which
/// `Request::Unrecognized` earns on the daemon side. `Unrecognized` covers a
/// code this build was not taught.
async fn resolved_dog_section<F, Fut>(name: &str, ask: F) -> Result<DogSectionToml, RequestError>
where
    F: Fn(Request) -> Fut,
    Fut: Future<Output = Result<Response, RequestError>>,
{
    match ask(dog_config_request(name)).await {
        Err(RequestError::Rpc(refusal))
            if matches!(
                refusal.code,
                RpcErrorCode::Unsupported | RpcErrorCode::Unrecognized
            ) =>
        {
            let plain = Request::DogConfig {
                name: name.to_owned(),
            };
            let section = dog_section(ask(plain).await?)?;
            if !references_in(section.as_str()).is_empty() {
                return Err(RequestError::Rpc(RpcError {
                    code: RpcErrorCode::Unsupported,
                    message: format!(
                        "the shepherd is too old to resolve secret references in the \
                         `{name}` dog's section"
                    ),
                    daemon_version: refusal.daemon_version,
                }));
            }
            Ok(section)
        }
        answer => dog_section(answer?),
    }
}

fn dog_section(answer: Response) -> Result<DogSectionToml, RequestError> {
    match answer {
        Response::DogSection { toml } => Ok(toml),
        other => Err(unexpected("DogConfigResolved", &other)),
    }
}

fn flock(answer: Response) -> Result<Vec<ProcessInfo>, RequestError> {
    match answer {
        Response::Flock(flock) => Ok(flock),
        other => Err(unexpected("ListFlock", &other)),
    }
}

fn reopened(answer: Response) -> Result<Vec<ProcessInfo>, RequestError> {
    match answer {
        Response::Reopened(sheep) => Ok(sheep),
        other => Err(unexpected("Reopen", &other)),
    }
}

fn host(answer: Response) -> Result<Option<HostUsage>, RequestError> {
    match answer {
        Response::HostUsage(usage) => Ok(usage),
        other => Err(unexpected("HostUsage", &other)),
    }
}

fn unexpected(asked: &'static str, answer: &Response) -> RequestError {
    RequestError::UnexpectedReply {
        asked,
        answered: answer.name(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use shep_core::protocol::{Envelope, HostUsage, RpcError, RpcErrorCode};
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::*;
    use crate::testing::{
        Handovers, Handshake, control_address, fake_client_answering, fake_daemon_across_handovers,
        sample_ack, sample_info, serve_one_request,
    };

    /// The sentinel stands for a secret's value: it must reach no error text.
    const SENTINEL: &str = "sentinel-value-1f3a";

    /// What a shepherd that has never heard of the request answers.
    fn does_not_know_the_request() -> RpcError {
        RpcError {
            code: RpcErrorCode::Unsupported,
            message: "this shepherd does not implement the request".to_owned(),
            daemon_version: None,
        }
    }

    /// A shepherd built before `DogConfigResolved`, whose section is `section`.
    fn older_shepherd(path: &std::path::Path, section: &str) -> Handovers {
        let shepherd = fake_daemon_across_handovers(path, vec![Handshake::Accept(sample_ack())]);
        shepherd.reply_to_dog_config(section);
        shepherd.refuse_dog_config_resolved(does_not_know_the_request());
        shepherd
    }

    /// The request bodies the shepherd received, in order.
    fn asked(shepherd: &Handovers) -> Vec<Request> {
        shepherd
            .envelopes()
            .into_iter()
            .map(|(_, envelope)| envelope.body)
            .collect()
    }

    fn named(name: &str) -> (Request, Request) {
        (
            Request::DogConfigResolved {
                name: name.to_owned(),
            },
            Request::DogConfig {
                name: name.to_owned(),
            },
        )
    }

    fn too_old_refusal(name: &str) -> RequestError {
        RequestError::Rpc(RpcError {
            code: RpcErrorCode::Unsupported,
            message: format!(
                "the shepherd is too old to resolve secret references in the `{name}` dog's section"
            ),
            daemon_version: None,
        })
    }

    /// The one envelope the client under test sent.
    async fn sent(envelopes: &mut UnboundedReceiver<Envelope>) -> Envelope {
        tokio::time::timeout(Duration::from_secs(5), envelopes.recv())
            .await
            .expect("the request reached the fake daemon")
            .expect("the fake daemon is still up")
    }

    #[tokio::test]
    async fn dog_config_asks_by_name_and_answers_the_section() {
        let dir = tempfile::tempdir().unwrap();
        let (client, mut envelopes) =
            fake_client_answering(&control_address(dir.path()), |_| Response::DogSection {
                toml: "poll = \"1m\"\n".to_owned().into(),
            })
            .await;

        let section = client.dog_config("bark").await.unwrap();

        assert_eq!(section.as_str(), "poll = \"1m\"\n");
        assert_eq!(
            sent(&mut envelopes).await.body,
            Request::DogConfigResolved {
                name: "bark".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn list_flock_answers_the_listing() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _envelopes) = fake_client_answering(&control_address(dir.path()), |_| {
            Response::Flock(vec![sample_info()])
        })
        .await;

        assert_eq!(client.list_flock().await.unwrap(), vec![sample_info()]);
    }

    /// The log plane walks the flock file by file, so the default budget
    /// can run out on a slow disk.
    #[tokio::test]
    async fn reopen_names_the_selector_and_asks_for_the_log_plane_budget() {
        let dir = tempfile::tempdir().unwrap();
        let (client, mut envelopes) = fake_client_answering(&control_address(dir.path()), |_| {
            Response::Reopened(vec![sample_info()])
        })
        .await;

        let reopened = client
            .reopen(SelectorSpec::Name("web".to_owned()))
            .await
            .unwrap();

        assert_eq!(reopened, vec![sample_info()]);
        let envelope = sent(&mut envelopes).await;
        assert_eq!(
            envelope.body,
            Request::Reopen {
                selector: SelectorSpec::Name("web".to_owned())
            }
        );
        let budget = u64::try_from(LOG_PLANE_DEADLINE.as_millis()).unwrap();
        assert_eq!(envelope.deadline_ms, Some(budget));
    }

    #[tokio::test]
    async fn host_usage_answers_the_reading() {
        let dir = tempfile::tempdir().unwrap();
        let reading = HostUsage {
            cpu_percent: None,
            memory_used_bytes: 1,
            memory_total_bytes: 2,
            disk_bytes_per_second: None,
            network_bytes_per_second: None,
        };
        let (client, _envelopes) = fake_client_answering(&control_address(dir.path()), move |_| {
            Response::HostUsage(Some(reading))
        })
        .await;

        assert_eq!(client.host_usage().await.unwrap(), Some(reading));
    }

    /// The error names both variants and never the section's body.
    #[tokio::test]
    async fn another_answer_is_named_by_variant_alone() {
        let dir = tempfile::tempdir().unwrap();
        let secret = "https://example.invalid/hook/super-secret-token";
        let (client, _envelopes) = fake_client_answering(&control_address(dir.path()), move |_| {
            Response::DogSection {
                toml: format!("webhook = \"{secret}\"\n").into(),
            }
        })
        .await;

        let err = client.list_flock().await.unwrap_err();

        assert_eq!(
            err,
            RequestError::UnexpectedReply {
                asked: "ListFlock",
                answered: "DogSection",
            }
        );
        assert_eq!(
            err.to_string(),
            "the daemon answered ListFlock with DogSection"
        );
    }

    #[tokio::test]
    async fn a_reconnecting_client_asks_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        let socket = control_address(dir.path());
        let served = serve_one_request(
            &socket,
            sample_ack(),
            Response::DogSection {
                toml: String::new().into(),
            },
        )
        .await;
        let client = ReconnectingClient::connect(&socket).await.unwrap();

        let section = client.dog_config("metrics").await.unwrap();

        assert_eq!(section.as_str(), "");
        let envelope = tokio::time::timeout(Duration::from_secs(5), served)
            .await
            .expect("the request reached the fake daemon")
            .unwrap();
        assert_eq!(
            envelope.body,
            Request::DogConfigResolved {
                name: "metrics".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn an_older_shepherd_that_does_not_know_the_request_is_asked_the_plain_one_when_the_section_names_no_reference()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = control_address(dir.path());
        let shepherd = older_shepherd(&path, "poll = \"1m\"\n");
        let client = Client::connect(&path).await.unwrap();

        let section = client.dog_config("bark").await.unwrap();

        assert_eq!(section.as_str(), "poll = \"1m\"\n");
        let (resolved, plain) = named("bark");
        assert_eq!(asked(&shepherd), vec![resolved, plain]);
    }

    #[tokio::test]
    async fn an_older_shepherd_with_a_reference_in_the_section_is_refused_without_the_section_text()
    {
        let dir = tempfile::tempdir().unwrap();
        let path = control_address(dir.path());
        let _shepherd = older_shepherd(&path, &format!("token = \"{{{{secret:{SENTINEL}}}}}\"\n"));
        let client = Client::connect(&path).await.unwrap();

        let err = client.dog_config("bark").await.unwrap_err();

        assert_eq!(err, too_old_refusal("bark"));
        let text = format!("{err} {err:?}");
        assert!(!text.contains(SENTINEL), "{text}");
        assert!(!text.contains("{{secret:"), "{text}");
    }

    /// An escaped `{{{{secret:` and a malformed token name no reference, so
    /// the plain section is safe to hand over.
    #[tokio::test]
    async fn an_older_shepherd_with_only_text_that_looks_like_a_reference_is_read_as_before() {
        for text in [
            "note = \"{{{{secret:X}}}}\"\n",
            "note = \"{{secret:bad key}}\"\n",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = control_address(dir.path());
            let _shepherd = older_shepherd(&path, text);
            let client = Client::connect(&path).await.unwrap();

            let section = client.dog_config("bark").await.unwrap();

            assert_eq!(section.as_str(), text);
        }
    }

    #[tokio::test]
    async fn a_reconnecting_client_on_an_older_shepherd_asks_the_plain_one_when_the_section_names_no_reference()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = control_address(dir.path());
        let shepherd = older_shepherd(&path, "poll = \"1m\"\n");
        let client = ReconnectingClient::connect(&path).await.unwrap();

        let section = client.dog_config("metrics").await.unwrap();

        assert_eq!(section.as_str(), "poll = \"1m\"\n");
        let (resolved, plain) = named("metrics");
        assert_eq!(asked(&shepherd), vec![resolved, plain]);
    }

    #[tokio::test]
    async fn a_reconnecting_client_on_an_older_shepherd_with_a_reference_is_refused_without_the_section_text()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = control_address(dir.path());
        let _shepherd = older_shepherd(&path, &format!("token = \"{{{{secret:{SENTINEL}}}}}\"\n"));
        let client = ReconnectingClient::connect(&path).await.unwrap();

        let err = client.dog_config("metrics").await.unwrap_err();

        assert_eq!(err, too_old_refusal("metrics"));
        let text = format!("{err} {err:?}");
        assert!(!text.contains(SENTINEL), "{text}");
        assert!(!text.contains("{{secret:"), "{text}");
    }
}
