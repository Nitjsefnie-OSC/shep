//! The dog connections this shepherd is serving, kept so a handover can carry
//! them.
//!
//! The server task that reads a connection owns its descriptor, and nothing
//! else can name it. This table holds a duplicate of each, plus the two facts
//! about the connection the exec would otherwise erase: whether the dog has
//! said `Hello`, and what it subscribed to.
//!
//! The serving tasks keep running while a handover takes its snapshot, so the
//! table has a freeze. [`DogLinks::freeze`] copies the rows and from then on
//! holds every task that would change one, until [`DogLinks::thaw`]. A task
//! records a change before it replies, so a row never says less than the dog
//! has been told, and it never changes after the copy. What the dog sent in
//! the gap goes unanswered and is lost with the exec, as any request in
//! flight is.

use std::collections::BTreeMap;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::Notify;

use crate::handover::DogConnection;

/// What the table knows about one connection.
#[derive(Debug)]
struct Held {
    name: String,
    pid: u32,
    fd: OwnedFd,
    /// `None` until the dog's `Hello`; then its topic patterns.
    topics: Option<Vec<String>>,
}

/// The table's shared state: a key per connection, so two connections of one
/// dog (the old and the new across a respawn) never share a row.
#[derive(Debug, Default)]
struct Rows {
    /// Set by [`DogLinks::freeze`]: no row changes until it clears.
    frozen: bool,
    next: u64,
    held: BTreeMap<u64, Held>,
}

/// Every dog connection being served, by lease
///
/// Cheap to clone: the clones share one table. `Debug` is derived and prints
/// names, pids, descriptor numbers and topic patterns, all of which are in
/// `shep flock` already.
#[derive(Debug, Clone, Default)]
pub(crate) struct DogLinks {
    rows: Arc<Mutex<Rows>>,
    /// Woken when the freeze ends.
    thawed: Arc<Notify>,
}

impl DogLinks {
    /// Enters the connection `stream` into the table and returns the lease
    /// that keeps it there.
    ///
    /// `resumed` is the topics of a connection a predecessor already greeted,
    /// and `None` for a fresh one. A duplicate that cannot be made (out of
    /// descriptors) leaves the connection served and untracked: a handover
    /// then leaves it out and the dog falls back to the control socket.
    pub(super) fn register(
        &self,
        name: &str,
        pid: u32,
        stream: &impl AsFd,
        resumed: Option<Vec<String>>,
    ) -> Lease {
        let fd = match stream.as_fd().try_clone_to_owned() {
            Ok(fd) => fd,
            Err(error) => {
                tracing::warn!(dog = name, %error, "a dog connection could not be tracked for a handover");
                return Lease::untracked();
            }
        };
        let mut rows = self.lock();
        let key = rows.next;
        rows.next += 1;
        rows.held.insert(
            key,
            Held {
                name: name.to_owned(),
                pid,
                fd,
                topics: resumed,
            },
        );
        Lease {
            tracked: Some((self.clone(), key)),
        }
    }

    /// Freezes the table and returns a fresh duplicate of every connection
    /// being served, for a handover to carry.
    ///
    /// Duplicates rather than the table's own descriptors, so the numbers the
    /// blob names stay valid for as long as the caller holds these, whatever
    /// happens to the connections meanwhile. The caller that does not exec
    /// calls [`thaw`](Self::thaw).
    pub(crate) fn freeze(&self) -> Vec<DogConnection> {
        let mut rows = self.lock();
        rows.frozen = true;
        Self::copy(&rows)
    }

    /// Ends a freeze, for a handover that was refused: the tasks it held
    /// carry on.
    pub(crate) fn thaw(&self) {
        self.lock().frozen = false;
        self.thawed.notify_waiters();
    }

    /// [`freeze`](Self::freeze) without the freeze.
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> Vec<DogConnection> {
        Self::copy(&self.lock())
    }

    fn copy(rows: &Rows) -> Vec<DogConnection> {
        rows.held
            .values()
            .filter_map(|held| {
                Some(DogConnection {
                    name: held.name.clone(),
                    pid: held.pid,
                    fd: held.fd.try_clone().ok()?,
                    topics: held.topics.clone(),
                })
            })
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Rows> {
        self.rows.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// One connection's place in the [`DogLinks`] table, removed when dropped
///
/// Held by the task serving the connection, so the row lives exactly as long
/// as the serving does. `Debug` is derived: a key and a handle to the table.
#[derive(Debug)]
pub(super) struct Lease {
    tracked: Option<(DogLinks, u64)>,
}

impl Lease {
    /// A lease on nothing, for a connection that is not being tracked.
    pub(super) const fn untracked() -> Self {
        Self { tracked: None }
    }

    /// Records that the dog has said `Hello`, so a successor treats its next
    /// frame as a request. Before the reply, and held through a freeze.
    pub(super) async fn greeted(&self) {
        self.update(|topics| {
            topics.get_or_insert_with(Vec::new);
        })
        .await;
    }

    /// Records the dog's current subscription, replacing the last: a
    /// connection has one topic list. Before the reply, and held through a
    /// freeze.
    pub(super) async fn subscribed(&self, patterns: &[String]) {
        self.update(|topics| *topics = Some(patterns.to_vec()))
            .await;
    }

    async fn update(&self, change: impl FnOnce(&mut Option<Vec<String>>)) {
        let Some((links, key)) = &self.tracked else {
            return;
        };
        let mut change = Some(change);
        loop {
            // Registered before the check, so a thaw between the two is not
            // missed.
            let thawed = links.thawed.notified();
            tokio::pin!(thawed);
            thawed.as_mut().enable();
            {
                let mut rows = links.lock();
                if !rows.frozen {
                    if let (Some(held), Some(change)) = (rows.held.get_mut(key), change.take()) {
                        change(&mut held.topics);
                    }
                    return;
                }
            }
            thawed.await;
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some((links, key)) = &self.tracked {
            links.lock().held.remove(key);
        }
    }
}

#[cfg(test)]
mod tests;
