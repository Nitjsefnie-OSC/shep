use super::*;
use shep_core::protocol::codec;

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn a_fresh_codec_holds_nothing() {
    assert!(!TrackedCodec::new(codec()).holding());
}

#[test]
fn a_header_without_its_payload_is_holding_though_the_buffer_is_empty() {
    let mut codec = TrackedCodec::new(codec());
    let mut src = BytesMut::from(&frame(b"hello")[..4]);

    assert!(codec.decode(&mut src).unwrap().is_none());

    assert!(src.is_empty(), "the header was consumed");
    assert!(codec.holding());
}

#[test]
fn half_a_header_is_holding() {
    let mut codec = TrackedCodec::new(codec());
    let mut src = BytesMut::from(&frame(b"hello")[..2]);

    assert!(codec.decode(&mut src).unwrap().is_none());

    assert!(codec.holding());
}

#[test]
fn a_whole_frame_leaves_nothing_held() {
    let mut codec = TrackedCodec::new(codec());
    let mut src = BytesMut::from(&frame(b"hello")[..]);

    assert!(codec.decode(&mut src).unwrap().is_some());

    assert!(!codec.holding());
}

#[test]
fn a_frame_with_the_start_of_the_next_behind_it_is_holding() {
    let mut codec = TrackedCodec::new(codec());
    let mut bytes = frame(b"hello");
    bytes.extend_from_slice(&frame(b"again")[..3]);
    let mut src = BytesMut::from(&bytes[..]);

    assert!(codec.decode(&mut src).unwrap().is_some());

    assert!(codec.holding());
}

#[test]
fn the_payload_arriving_ends_the_holding() {
    let mut codec = TrackedCodec::new(codec());
    let bytes = frame(b"hello");
    let mut src = BytesMut::from(&bytes[..6]);
    assert!(codec.decode(&mut src).unwrap().is_none());
    assert!(codec.holding());

    src.extend_from_slice(&bytes[6..]);

    assert!(codec.decode(&mut src).unwrap().is_some());
    assert!(!codec.holding());
}
