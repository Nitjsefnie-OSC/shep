//! Serving the connection each dog is spawned holding.

use tokio::sync::{mpsc, watch};

use super::conn_protocol::serve_conn;
use super::origin::Origin;
use crate::rpc::RpcContext;
use crate::runner::DogLink;

/// Serves every [`DogLink`] a runner opens until `shutdown` flips or the
/// runner is gone
///
/// Independent of the control socket's listener: the links arrive at spawn, so
/// a dog started ahead of the muster restore is served during it. Each link
/// runs the same handshake and request loop as a socket connection, with the
/// identity the runner gave it in place of the peer's claim.
pub(crate) async fn serve_dog_links(
    mut links: mpsc::UnboundedReceiver<DogLink>,
    ctx: RpcContext,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            link = links.recv() => {
                let Some(DogLink { name, pid, stream }) = link else { return };
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let origin = Origin::Dog { name, pid };
                    if let Err(err) = serve_conn(stream, ctx, origin).await {
                        tracing::debug!(%err, "dog connection ended");
                    }
                });
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return;
                }
            }
        }
    }
}
