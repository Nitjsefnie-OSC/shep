//! A dog's own connection through the rehearsal and the adoption.

use std::io::{Read as _, Write as _};

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::dry_run;
use super::testing::*;

#[tokio::test]
async fn a_blob_carrying_a_dog_connection_passes_the_rehearsal() {
    let predecessor = Predecessor::new();

    dry_run(&predecessor.blob_with_dog_link()).expect("the fixture is adoptable");
}

#[tokio::test]
async fn a_rehearsal_leaves_the_dog_connection_working() {
    let mut predecessor = Predecessor::new();

    dry_run(&predecessor.blob_with_dog_link()).expect("the fixture is adoptable");

    predecessor.dog_link.write_all(b"ping").unwrap();
    let mut buf = [0_u8; 4];
    predecessor
        .child_dog_link
        .read_exact(&mut buf)
        .expect("the checked connection still reaches the dog");
    assert_eq!(&buf, b"ping");
}

#[tokio::test]
async fn an_adopted_dog_connection_is_the_dogs_own_socket() {
    let mut predecessor = Predecessor::new();

    let mut adopted = adopt_a_copy_keeping(&predecessor.blob_with_dog_link()).unwrap();
    let link = adopted.sheep[0]
        .dog_link
        .as_mut()
        .expect("a carried connection is adopted");

    predecessor.child_dog_link.write_all(b"next").unwrap();
    let mut buf = [0_u8; 4];
    link.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"next", "the dog's next frame reaches the successor");

    link.write_all(b"back").await.unwrap();
    let mut buf = [0_u8; 4];
    predecessor.child_dog_link.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"back", "and the successor's reply reaches the dog");
}

#[tokio::test]
async fn what_the_dog_had_subscribed_to_arrives_with_the_connection() {
    let predecessor = Predecessor::new();

    let adopted = adopt_a_copy_keeping(&predecessor.blob_with_dog_link()).unwrap();

    assert_eq!(
        adopted.sheep[0].carried.dog_topics(),
        Some(&["process.*".to_owned()][..])
    );
}

#[tokio::test]
async fn a_sheep_carrying_no_dog_connection_adopts_none() {
    let predecessor = Predecessor::new();

    let adopted = adopt_a_copy_keeping(&predecessor.blob()).unwrap();

    assert!(adopted.sheep[0].dog_link.is_none());
}

#[tokio::test]
async fn a_dog_connection_that_is_not_a_connected_socket_is_refused() {
    use std::os::fd::AsRawFd as _;

    let predecessor = Predecessor::new();
    // A socket, and the one most likely to be mistaken for the connection:
    // nothing is on the far end of it. A second one, as a number named twice
    // is refused before the kind is looked at.
    let dir = tempfile::tempdir().unwrap();
    let listening = std::os::unix::net::UnixListener::bind(dir.path().join("other.sock")).unwrap();
    let mut blob = predecessor.blob_with_dog_link();
    blob.sheep[0].fds.dog_link = Some(listening.as_raw_fd());

    let err = adopt_a_copy_keeping(&blob).expect_err("a listening socket is not a connection");

    assert!(
        err.to_string().contains("dog connection"),
        "the refusal must name what was refused: {err}"
    );
}
