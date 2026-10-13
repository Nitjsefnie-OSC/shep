//! `Request::DogConfigResolved`: a dog's section with its `{{secret:...}}`
//! references replaced.

use shep_core::protocol::{DogSectionToml, RpcError, RpcErrorCode};
use shep_core::secrets::SecretView;

use crate::dogs::{DogError, resolve_section, section_references, store_unreadable};

use super::context::RpcContext;

/// `name`'s section, read fresh from `dogs.toml` and resolved in the
/// shepherd's own environment.
///
/// The raw read comes first, so the text is the operator's own, and the
/// store is read only when the section names a reference: a corrupt
/// `secrets.json` cannot break a dog that asks for none. A section with none
/// is still resolved, against an empty view, so a malformed `{{secret:` is
/// refused rather than handed to the dog as literal text.
///
/// Nothing here writes `dogs.toml`. Every refusal names the section, the
/// key path and the reference, never a value.
pub(super) fn resolved_section(ctx: &RpcContext, name: &str) -> Result<DogSectionToml, RpcError> {
    let refuse = |message: String| RpcError {
        code: RpcErrorCode::InvalidConfig,
        message,
        daemon_version: None,
    };
    let raw =
        crate::dogs::dog_section(&ctx.dogs_config, name).map_err(|err| refuse(err.to_string()))?;
    let view = if section_references(&raw).is_empty() {
        SecretView::empty(ctx.host_environment.clone())
    } else {
        let store = shep_core::secrets::all(&ctx.paths.secrets)
            .map_err(|err| refuse(format!("[{name}] {}", store_unreadable(&raw, &err))))?;
        SecretView::new(
            ctx.host_environment.clone(),
            store,
            ctx.provider_secrets.snapshot(),
        )
    };
    resolve_section(&raw, &view).map_err(|err| match err {
        DogError::Reference { .. } => refuse(format!("[{name}] {err}")),
        other => refuse(other.to_string()),
    })
}
