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

#[tokio::test]
async fn a_connection_is_ungreeted_until_the_dog_says_hello() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);
    assert_eq!(links.snapshot()[0].topics, None);

    lease.greeted().await;

    assert_eq!(links.snapshot()[0].topics, Some(vec![]));
}

#[tokio::test]
async fn a_second_subscribe_replaces_the_first() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);

    lease.subscribed(&["process.*".to_owned()]).await;
    lease.subscribed(&["log.out".to_owned()]).await;

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

#[tokio::test]
async fn an_untracked_lease_changes_nothing() {
    let lease = Lease::untracked();
    lease.greeted().await;
    lease.subscribed(&["x".to_owned()]).await;
}

#[tokio::test]
async fn a_frozen_table_holds_a_greeting_until_it_thaws() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);

    let carried = links.freeze();
    let greeting = tokio::spawn(async move {
        lease.greeted().await;
        lease
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    assert_eq!(carried[0].topics, None, "the copy is what the blob carries");
    assert_eq!(links.snapshot()[0].topics, None, "the row did not move");
    assert!(
        !greeting.is_finished(),
        "the task waits rather than replying"
    );

    links.thaw();
    let _lease = greeting.await.unwrap();
    assert_eq!(links.snapshot()[0].topics, Some(vec![]));
}

#[tokio::test]
async fn a_change_made_before_the_freeze_is_in_the_copy() {
    let links = DogLinks::default();
    let stream = socket();
    let lease = links.register("metrics", 41, &stream, None);

    lease.subscribed(&["process.*".to_owned()]).await;
    let carried = links.freeze();

    assert_eq!(carried[0].topics, Some(vec!["process.*".to_owned()]));
}
