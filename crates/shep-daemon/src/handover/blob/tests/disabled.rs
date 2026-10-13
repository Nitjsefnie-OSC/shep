//! The disabled mark on a carried sheep: it crosses the blob, and a blob an
//! older image wrote has no key for it.

use super::sample_handover;
use crate::handover::Handover;

#[test]
fn a_disabled_mark_crosses_the_blob() {
    let mut h = sample_handover();
    h.sheep[0] = h.sheep[0].clone().with_disabled(true);

    let back: Handover = serde_json::from_str(&serde_json::to_string(&h).unwrap()).unwrap();

    assert_eq!(back.sheep[0].disabled(), Some(true));
}

#[test]
fn a_blob_written_before_the_disabled_mark_was_carried_still_loads() {
    let mut h = sample_handover();
    h.sheep[0] = h.sheep[0].clone().with_disabled(true);
    let mut value = serde_json::to_value(h).unwrap();
    let sheep = value["sheep"][0]
        .as_object_mut()
        .expect("a carried sheep is an object");
    assert!(
        sheep.remove("disabled").is_some(),
        "the field this case removes must be there to remove"
    );

    let loaded = Handover::load_value(value).expect("an older blob must still load");

    assert_eq!(loaded.sheep[0].disabled(), None);
}
