//! `sort_flock`'s tie-breaks on an instance slot.
//!
//! Sits beside [`super`] rather than inside it because `process.rs` is at
//! IR-48's file-size ceiling.

use super::*;

#[test]
fn sort_flock_orders_by_slot_before_id() {
    // A reload gave slot 0 a fresh, higher id. Slot order must still win.
    let mut listing = vec![
        ProcessInfo::builder(9, "web", ProcStatus::Online)
            .instance(Some(0))
            .build(),
        ProcessInfo::builder(2, "web", ProcStatus::Online)
            .instance(Some(1))
            .build(),
    ];
    sort_flock(&mut listing);
    assert_eq!(
        listing.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![9, 2],
        "slot 0 leads even though its id is higher"
    );
}

#[test]
fn sort_flock_falls_back_to_id_when_no_row_carries_a_slot() {
    let mut listing = vec![
        ProcessInfo::builder(5, "web", ProcStatus::Online).build(),
        ProcessInfo::builder(3, "web", ProcStatus::Online).build(),
    ];
    sort_flock(&mut listing);
    assert_eq!(
        listing.iter().map(|i| i.id).collect::<Vec<_>>(),
        vec![3, 5],
        "an older daemon's listing sorts exactly as it does today"
    );
}
