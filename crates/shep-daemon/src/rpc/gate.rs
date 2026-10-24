//! The check on a dog's own connection: whether the dog was granted what
//! the request takes.
//!
//! [`check`] asks `shep_core::permissions::need` what a request takes and
//! looks up what only the shepherd can: which sheep or dog a selector
//! reaches, and which environment a sheep resolves its secrets in. The
//! control socket never comes through here; it acts as the operator.

use shep_core::permissions::{Grants, Need, Permission, need};
use shep_core::protocol::{Request, RpcError, RpcErrorCode};
use shep_core::selector::ProcessSelector;

use super::context::RpcContext;
use super::error::rpc_error;

/// Whether the dog `dog` may send `request`.
///
/// A dog with no grants recorded is undeclared and keeps full access.
///
/// # Errors
/// - [`RpcErrorCode::Forbidden`] naming the dog, the request and the first
///   permission it lacks.
/// - Whatever the supervisor reports, when it cannot answer the lookup.
pub(super) async fn check(ctx: &RpcContext, dog: &str, request: &Request) -> Result<(), RpcError> {
    let Some(grants) = ctx.known_dogs.grants(dog) else {
        return Ok(());
    };
    let refuse = |needed: &str| forbidden(dog, request, needed);
    match need(request, dog) {
        Need::Always => Ok(()),
        Need::Permission(permission) => require(&grants, &permission).map_err(|p| refuse(&p)),
        Need::Selector {
            selector,
            sheep,
            dog: for_dog,
        } => {
            // An invalid selector is the handler's to refuse, with its own
            // reason.
            let Ok(selector) = ProcessSelector::try_from(selector.clone()) else {
                return Ok(());
            };
            let infos = ctx
                .supervisor
                .list_checked()
                .await
                .map_err(|err| rpc_error(&err))?;
            // The rule every lifecycle verb applies: a sweep passes a dog
            // by and an exact selector reaches it.
            let exact = selector.is_exact();
            let mut reached = infos
                .iter()
                .filter(|i| exact || i.dog.is_none())
                .filter(|i| selector.matches(&i.name, i.id, i.fold.as_deref(), i.instance))
                .map(|i| {
                    if i.dog.is_some() {
                        for_dog(&i.name)
                    } else {
                        sheep.clone()
                    }
                })
                .peekable();
            // Reaching nothing takes the sheep permission, so a dog cannot
            // probe for what exists.
            if reached.peek().is_none() {
                return require(&grants, &sheep).map_err(|p| refuse(&p));
            }
            reached.try_for_each(|p| require(&grants, &p).map_err(|p| refuse(&p)))
        }
        Need::Topics(topics) => topics
            .iter()
            .find(|topic| !grants.allows_topic(topic) && !is_own_config_topic(topic, dog))
            .map_or(Ok(()), |topic| {
                Err(refuse(&format!("events:{}", literal_head(topic))))
            }),
        Need::ResolvedRead { sheep, dog: table } => {
            let read = ctx
                .supervisor
                .sheep_dog_read(sheep.to_string(), table.to_string())
                .await
                .map_err(|err| rpc_error(&err))?;
            // No such sheep: the handler answers it, and there is no
            // environment to have refused.
            let Some(read) = read else { return Ok(()) };
            require(&grants, &Permission::SecretsRead(read.environment)).map_err(|p| refuse(&p))
        }
        // `Need` is `#[non_exhaustive]`: a need this build cannot read is
        // refused rather than waved through.
        _ => Err(refuse("a permission this shepherd does not know")),
    }
}

/// `Ok` when `grants` holds `permission`, and its name otherwise.
fn require(grants: &Grants, permission: &Permission) -> Result<(), String> {
    if grants.allows(permission) {
        Ok(())
    } else {
        Err(permission.to_string())
    }
}

/// The two topics that announce a change to a dog's own section and tables,
/// which the dog may always read, as it may always read the section.
fn is_own_config_topic(topic: &str, dog: &str) -> bool {
    topic
        .strip_prefix("config.dog.")
        .or_else(|| topic.strip_prefix("config.sheep."))
        == Some(dog)
}

/// The wire name of `request`'s variant, and nothing of what it carries.
///
/// A request that serializes with no `kind` is still narrated, as `request`.
pub(crate) fn kind_of(request: &Request) -> String {
    serde_json::to_value(request)
        .ok()
        .and_then(|value| value.get("kind")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "request".to_owned())
}

/// The part of a topic glob before its first glob character: the `events:`
/// prefix a grant must cover to allow it.
fn literal_head(pattern: &str) -> &str {
    pattern
        .find(['*', '?', '[', '{', '\\'])
        .map_or(pattern, |at| &pattern[..at])
}

fn forbidden(dog: &str, request: &Request, needed: &str) -> RpcError {
    RpcError {
        code: RpcErrorCode::Forbidden,
        message: format!(
            "dog `{dog}` was refused `{}`: it needs `{needed}`, which was not granted to it",
            kind_of(request)
        ),
        daemon_version: None,
    }
}
