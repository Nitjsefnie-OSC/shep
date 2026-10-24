use std::time::Duration;

use shep_core::transport::connect;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

use super::*;

/// Every await bounded, so a hang fails this test and not the whole binary.
const BOUND: Duration = Duration::from_secs(10);

/// fails if two binds of one kind share a name, which would let the second
/// spawn's pipe collide with the first's.
#[tokio::test]
async fn each_bind_names_a_pipe_of_its_own() {
    let (first, _a) = bind("dog", "test pipe").unwrap();
    let (second, _b) = bind("dog", "test pipe").unwrap();

    assert_ne!(first, second);
    assert!(first.to_string_lossy().starts_with(r"\\.\pipe\shep-dog-"));
}

/// fails if the dog's connection does not reach the links channel, carrying
/// the name and pid it was spawned under, with the dog's bytes behind it.
#[tokio::test]
async fn a_dog_that_opens_the_pipe_is_handed_over_as_a_link() {
    let (pipe, listener) = bind("dog", "test pipe").unwrap();
    let (links, mut linked) = mpsc::unbounded_channel();
    let (keep, _kept) = mpsc::channel::<()>(1);
    serve_dog_link(listener, "metrics".to_string(), 77, links, keep);

    let mut client = connect(&pipe).await.unwrap();
    client.write_all(b"hi").await.unwrap();
    let mut link = tokio::time::timeout(BOUND, linked.recv())
        .await
        .expect("the link must arrive once the dog connects")
        .unwrap();

    assert_eq!((link.name.as_str(), link.pid), ("metrics", 77));
    assert!(link.resumed.is_none());
    let mut heard = [0_u8; 2];
    tokio::time::timeout(BOUND, link.stream.read_exact(&mut heard))
        .await
        .expect("the dog's bytes must arrive")
        .unwrap();
    assert_eq!(&heard, b"hi");
}

/// fails if a dog that never opens its pipe holds the listener after the dog
/// is gone.
#[tokio::test]
async fn a_dog_that_never_connects_frees_the_pipe_once_it_is_gone() {
    let (pipe, listener) = bind("dog", "test pipe").unwrap();
    let (links, mut linked) = mpsc::unbounded_channel();
    let (gone, kept) = mpsc::channel::<()>(1);
    serve_dog_link(listener, "metrics".to_string(), 77, links, gone);

    drop(kept);

    assert!(
        tokio::time::timeout(BOUND, linked.recv())
            .await
            .expect("the task must end and drop its sender")
            .is_none(),
        "no link arrives from a dog that never connected"
    );
    assert!(
        connect(&pipe).await.is_err(),
        "the pipe is gone with the listener"
    );
}
