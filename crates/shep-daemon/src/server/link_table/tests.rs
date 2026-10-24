use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;

use super::*;

fn socket() -> UnixStream {
    UnixStream::pair().unwrap().0
}

#[test]
fn a_registered_connection_is_snapshotted_until_its_lease_drops() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);

    let held = links.snapshot();
    assert_eq!(held.len(), 1);
    assert_eq!((held[0].name.as_str(), held[0].pid), ("metrics", 41));

    drop(lease);
    assert!(links.snapshot().is_empty());
}

#[test]
fn a_connection_is_ungreeted_until_the_dog_says_hello() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);
    assert_eq!(links.snapshot()[0].topics, None);

    lease.greeted();

    assert_eq!(links.snapshot()[0].topics, Some(vec![]));
}

#[test]
fn a_second_subscribe_replaces_the_first() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);

    lease.subscribed(&["process.*".to_owned()]);
    lease.subscribed(&["log.out".to_owned()]);

    assert_eq!(links.snapshot()[0].topics, Some(vec!["log.out".to_owned()]));
}

#[test]
fn a_resumed_connection_starts_with_what_its_predecessor_recorded() {
    let links = DogLinks::default();
    let stream = socket();
    let _lease = links.register("metrics", 41, &stream, Some(vec!["bark.*".to_owned()]));

    assert_eq!(links.snapshot()[0].topics, Some(vec!["bark.*".to_owned()]));
}

#[test]
fn two_connections_of_one_dog_are_two_rows() {
    let links = DogLinks::default();
    let (old, new) = (socket(), socket());
    let old_lease = links.register("metrics", 41, &old, None);
    let _new_lease = links.register("metrics", 42, &new, None);

    drop(old_lease);

    let held = links.snapshot();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].pid, 42);
}

#[test]
fn the_snapshot_keeps_its_descriptor_open_after_the_connection_ends() {
    let links = DogLinks::default();
    let stream = socket();
    let source = stream.as_raw_fd();
    let lease = links.register("metrics", 41, &stream, None);
    let held = links.snapshot();

    drop(lease);
    drop(stream);

    assert_ne!(held[0].fd.as_raw_fd(), source);
    let probe = held[0].fd.try_clone();
    assert!(probe.is_ok(), "the carried number must stay a live socket");
}

#[test]
fn an_untracked_lease_changes_nothing() {
    let lease = Lease::untracked();
    lease.greeted();
    lease.subscribed(&["x".to_owned()]);
}
