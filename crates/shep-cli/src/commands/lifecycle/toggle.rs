//! `enable` and `disable`: one verb pair for a sheep or a dog.
//!
//! A dog's name edits `shep.toml` and needs no shepherd. Anything else is a
//! sheep, which only a running shepherd can mark, so with none running the
//! verb refuses and does not start one.

use shep_client::{Client, RequestError};
use shep_core::paths::ShepPaths;
use shep_core::protocol::{Request, Response, RpcErrorCode, SelectorSpec};
use shep_core::selector::ProcessSelector;

use crate::commands::dogs;
use crate::commands::lifecycle::selectors::{flock_now, render_outcome};
use crate::commands::rpc::{client_error, unexpected_response};
use crate::commands::selector::parse_selector;
use crate::commands::shep_toml::ShepToml;
use crate::exit::ExitCode;
use crate::output::{FlockRows, Streams};

/// Which of the two verbs ran
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toggle {
    /// `shep enable`
    Enable,
    /// `shep disable`
    Disable,
}

impl Toggle {
    /// The verb as the operator types it
    fn verb(self) -> &'static str {
        match self {
            Self::Enable => "enable",
            Self::Disable => "disable",
        }
    }

    fn request(self, name: &str) -> Request {
        let selector = SelectorSpec::Name(name.to_string());
        match self {
            Self::Enable => Request::Enable { selector },
            Self::Disable => Request::Disable { selector },
        }
    }
}

/// `shep enable <name>` or `shep disable <name>`
///
/// A name the daemon config knows as a dog goes through the dog verbs. Any
/// other name is a sheep and is sent to the shepherd.
pub(crate) async fn toggle(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    verb: Toggle,
    name: &str,
) -> ExitCode {
    if names_a_dog(paths, name) {
        return match verb {
            Toggle::Enable => dogs::enable(streams, paths, name).await,
            Toggle::Disable => dogs::disable(streams, paths, name).await,
        };
    }
    let client = match dogs::connect_or_absent(paths, streams).await {
        Ok(Some(client)) => client,
        Ok(None) => {
            let message = format!(
                "no shepherd is running; `shep {} {name}` needs one to mark a sheep. \
                 start one with `shep start <target>`",
                verb.verb()
            );
            return streams.fail(ExitCode::DaemonUnreachable, &message);
        }
        Err(code) => return code,
    };
    toggle_sheep(&client, streams, verb, name).await
}

/// Whether `name` is a built-in dog, an adopted one or one already enabled
///
/// An unreadable config answers `false`: the sheep path then reports what
/// the shepherd says about the name.
fn names_a_dog(paths: &ShepPaths, name: &str) -> bool {
    if crate::dog::BUILT_IN_DOGS.contains(&name) {
        return true;
    }
    ShepToml::read_only(&paths.daemon_config).is_ok_and(|cfg| {
        cfg.adopted_dog_names().iter().any(|dog| dog == name)
            || cfg.enabled_dog_names().iter().any(|dog| dog == name)
    })
}

/// The sheep half of [`toggle`], against a connected shepherd
///
/// Only an exact app name is sent. An id or `name:slot` is refused here,
/// naming the app, so an operator never marks a whole app by naming one
/// instance of it.
pub(crate) async fn toggle_sheep(
    client: &Client,
    streams: &mut Streams<'_>,
    verb: Toggle,
    name: &str,
) -> ExitCode {
    let selector = match parse_selector(streams, name) {
        Ok(selector) => selector,
        Err(code) => return code,
    };
    let app = match selector {
        ProcessSelector::Name(app) => app,
        ProcessSelector::Instance { name: app, .. } => {
            return refuse_instance(streams, verb, name, &app);
        }
        ProcessSelector::Id(id) => {
            let flock = flock_now(client).await;
            return match flock.iter().find(|info| info.id == id) {
                Some(info) => refuse_instance(streams, verb, name, &info.name),
                None => streams.fail(ExitCode::NotFound, &format!("no sheep has id {id}")),
            };
        }
        _ => {
            let message = format!(
                "`shep {} {name}` is not supported yet; name one sheep, such as `shep {} api`",
                verb.verb(),
                verb.verb()
            );
            return streams.fail(ExitCode::Usage, &message);
        }
    };
    let response = match client.request(verb.request(&app)).await {
        Ok(response) => response,
        Err(RequestError::Rpc(err)) if err.code == RpcErrorCode::Unsupported => {
            let message = format!(
                "this shepherd is too old to {} a sheep; upgrade it with `shep {}`",
                verb.verb(),
                crate::version_guard::VERSION_SKEW_REMEDY
            );
            return streams.fail(ExitCode::Unsupported, &message);
        }
        Err(err) => return client_error(streams, &err),
    };
    match (verb, response) {
        (Toggle::Disable, Response::Disabled(procs)) => {
            render_outcome(client, streams, "disable", FlockRows(procs)).await
        }
        (Toggle::Enable, Response::Enabled(procs)) if procs.is_empty() => {
            streams.note(
                "not_disabled",
                &format!("`{app}` is not disabled; nothing changed"),
            );
            ExitCode::Success
        }
        (Toggle::Enable, Response::Enabled(procs)) => {
            render_outcome(client, streams, "enable", FlockRows(procs)).await
        }
        _ => unexpected_response(streams),
    }
}

/// Refuses an instance selector with the command that would work
fn refuse_instance(streams: &mut Streams<'_>, verb: Toggle, given: &str, app: &str) -> ExitCode {
    let message = format!(
        "`{given}` is one instance of `{app}`, and `{}` acts on a whole app; \
         run `shep {} {app}`",
        verb.verb(),
        verb.verb()
    );
    streams.fail(ExitCode::Usage, &message)
}
