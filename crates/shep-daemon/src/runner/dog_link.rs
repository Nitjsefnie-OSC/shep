//! The shepherd's end of a dog's own connection, as a runner hands it over.

use tokio::net::UnixStream;

/// The daemon's end of the connection a dog was spawned holding
///
/// Opened by the runner at spawn and named for the dog it belongs to, so the
/// server that serves it never asks the peer who it is: the connection is the
/// identity. Unix only, as the socketpair it wraps is.
///
/// `Debug` is derived: a name, a pid and a socket are all in `shep flock`
/// already, and no env value reaches this type.
#[derive(Debug)]
pub struct DogLink {
    /// The dog's name, from the spec that spawned it.
    pub name: String,
    /// The pid the dog was spawned under. A socketpair's peer credentials name
    /// the shepherd itself, so this is the only way to tell the silence watch
    /// which process just made contact.
    pub pid: u32,
    /// The shepherd's end, already connected to the dog's fd 4.
    pub stream: UnixStream,
}
