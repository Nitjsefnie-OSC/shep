//! The dog connections this shepherd is serving, kept so a handover can carry
//! them.
//!
//! The server task that reads a connection owns its descriptor, and nothing
//! else can name it. This table holds a duplicate of each, plus the two facts
//! about the connection the exec would otherwise erase: whether the dog has
//! said `Hello`, and what it subscribed to.

use std::collections::BTreeMap;
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Mutex, PoisonError};

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

    /// A fresh duplicate of every connection being served, for a handover to
    /// carry.
    ///
    /// Duplicates rather than the table's own descriptors, so the numbers the
    /// blob names stay valid for as long as the caller holds these, whatever
    /// happens to the connections meanwhile.
    pub(crate) fn snapshot(&self) -> Vec<DogConnection> {
        self.lock()
            .held
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
    /// frame as a request.
    pub(super) fn greeted(&self) {
        self.update(|topics| {
            topics.get_or_insert_with(Vec::new);
        });
    }

    /// Records the dog's current subscription, replacing the last: a
    /// connection has one topic list.
    pub(super) fn subscribed(&self, patterns: &[String]) {
        self.update(|topics| *topics = Some(patterns.to_vec()));
    }

    fn update(&self, change: impl FnOnce(&mut Option<Vec<String>>)) {
        let Some((links, key)) = &self.tracked else {
            return;
        };
        if let Some(held) = links.lock().held.get_mut(key) {
            change(&mut held.topics);
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
