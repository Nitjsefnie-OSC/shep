//! Who a connection is, as far as the daemon knows without asking the peer.

use std::collections::BTreeSet;

use shep_core::protocol::Request;

use crate::rpc::RpcContext;

/// How a connection reached the daemon
///
/// A connection on the control socket is whoever the uid check lets in, and
/// names itself in its `Hello`. A dog's own connection, opened at spawn, is
/// that dog and nobody else: the runner knew whom it handed the other end to.
#[derive(Debug, Clone)]
pub(super) enum Origin {
    /// Accepted on the control socket.
    Socket,
    /// The connection a dog was spawned holding.
    Dog {
        /// The dog's name, from the spec that spawned it.
        name: String,
        /// The pid it was spawned under.
        pid: u32,
    },
}

impl Origin {
    /// The dog this connection speaks for: the link's own name, or for the
    /// socket whatever the peer `claimed`.
    ///
    /// A claim on a link is dropped, so a dog cannot present as another.
    pub(super) fn dog<'a>(&'a self, claimed: Option<&'a str>) -> Option<&'a str> {
        match self {
            Self::Socket => claimed,
            Self::Dog { name, .. } => Some(name),
        }
    }

    /// The pid to record contact against: the link's, or for the socket the
    /// one the OS reported, which `None` on Windows.
    ///
    /// A socketpair's peer credentials name the daemon itself.
    pub(super) fn pid(&self, reported: Option<u32>) -> Option<u32> {
        match self {
            Self::Socket => reported,
            Self::Dog { pid, .. } => Some(*pid),
        }
    }

    /// Whether the control socket's same-uid check applies. A link needs none:
    /// only the process it was handed to holds the other end.
    pub(super) fn checks_peer(&self) -> bool {
        matches!(self, Self::Socket)
    }
}

/// Writes into a dog's own log what it asked of the shepherd
///
/// Once per kind of request per connection, so a dog polling every second
/// does not bury its own output in its own log.
#[derive(Debug, Default)]
pub(super) struct Attribution {
    dog: Option<String>,
    seen: BTreeSet<String>,
}

impl Attribution {
    /// Attributes requests to `origin`'s dog; a socket connection has none.
    pub(super) fn new(origin: &Origin) -> Self {
        match origin {
            Origin::Socket => Self::default(),
            Origin::Dog { name, .. } => Self {
                dog: Some(name.clone()),
                seen: BTreeSet::new(),
            },
        }
    }

    /// Narrates `request` into the dog's log if this connection has not made
    /// one of its kind before.
    pub(super) fn note(&mut self, ctx: &RpcContext, request: &Request) {
        let Some(dog) = &self.dog else { return };
        let Some(kind) = kind_of(request) else { return };
        if self.seen.insert(kind.clone()) {
            crate::dogs::narrate_by_name(
                &ctx.supervisor,
                &ctx.events,
                dog,
                format!("this dog sent shep a `{kind}` request on its own connection"),
            );
        }
    }
}

/// The wire name of `request`'s variant, and nothing of what it carries.
fn kind_of(request: &Request) -> Option<String> {
    let value = serde_json::to_value(request).ok()?;
    value.get("kind")?.as_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_ignores_the_name_the_peer_claims() {
        let link = Origin::Dog {
            name: "metrics".to_string(),
            pid: 7,
        };
        assert_eq!(link.dog(Some("bark")), Some("metrics"));
        assert_eq!(link.dog(None), Some("metrics"));
    }

    #[test]
    fn the_socket_takes_the_claim_as_given() {
        assert_eq!(Origin::Socket.dog(Some("bark")), Some("bark"));
        assert_eq!(Origin::Socket.dog(None), None);
    }

    #[test]
    fn a_link_records_contact_against_its_own_pid() {
        let link = Origin::Dog {
            name: "metrics".to_string(),
            pid: 7,
        };
        assert_eq!(link.pid(Some(99)), Some(7));
        assert_eq!(Origin::Socket.pid(Some(99)), Some(99));
    }

    #[test]
    fn only_the_socket_checks_its_peer() {
        assert!(Origin::Socket.checks_peer());
        assert!(
            !Origin::Dog {
                name: "metrics".to_string(),
                pid: 7
            }
            .checks_peer()
        );
    }

    #[test]
    fn a_request_is_named_by_its_variant_alone() {
        assert_eq!(kind_of(&Request::Ping).as_deref(), Some("ping"));
        assert_eq!(kind_of(&Request::ListFlock).as_deref(), Some("list_flock"));
    }
}
