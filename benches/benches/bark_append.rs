//! Times the one blocking file write the bark dog parks on a tokio worker:
//! [`shep_core::barks::append`] as `deliver_and_record`
//! (`shep-cli/src/dog/bark/delivery.rs`) calls it.
//!
//! `append` takes a `FileLock`, reads the whole ring, rewrites survivors to
//! a staging file, `fsync`s it, `rename`s it over the original and
//! `fsync`s the directory: work proportional to the ring, with two
//! `fsync`s, inline in an `async fn`.
//!
//! `full_ring` appends one bark into a ring already at
//! [`shep_core::barks::DEFAULT_MAX_BYTES`], which evicts one record;
//! `empty_ring` appends into a fresh file, so the fixed `fsync` cost reads
//! separately from the cost that grows with the ring. The ring is seeded
//! by writing `bark_for`-shaped lines directly: seeding through `append`
//! would rewrite the ring built so far on every one of 6,000-odd calls.
//!
//! Cost is machine-bound and not deterministic: recorded, not asserted. CI
//! runs with `-- --test`, executing each benchmark once and asserting
//! nothing.

use std::hint::black_box;
use std::io::Write as _;
use std::path::Path;

use criterion::{Criterion, criterion_group, criterion_main};
use shep_core::barks::{self, Bark, DEFAULT_MAX_BYTES, SinkOutcome};

/// A record of the shape `barks.rs`'s tests write, copied here so the
/// seeded ring holds the line width a real trail carries.
fn bark_for(at_ms: u64) -> Bark {
    Bark {
        at_ms,
        rule: "watchdog".to_owned(),
        subject: "web".to_owned(),
        message: "restart budget exhausted".to_owned(),
        sinks: vec![SinkOutcome {
            sink: "discord".to_owned(),
            error: None,
        }],
    }
}

/// Writes `path` as a ring of `bark_for` lines packed under `cap`.
///
/// Done directly rather than through `append`: seeding 6,000-odd records
/// one `append` at a time would rewrite the whole ring on every call, and
/// the seed would measure its own quadratic cost instead of setting up a
/// full ring to measure.
fn seed_ring(path: &Path, cap: u64) {
    let mut file = std::fs::File::create(path).unwrap();
    let mut bytes = 0u64;
    let mut at_ms = 0u64;
    loop {
        let line = serde_json::to_string(&bark_for(at_ms)).unwrap();
        // The `+ 1` is the newline `write_ring` puts after each line.
        let line_bytes = line.len() as u64 + 1;
        if bytes + line_bytes > cap {
            break;
        }
        file.write_all(line.as_bytes()).unwrap();
        file.write_all(b"\n").unwrap();
        bytes += line_bytes;
        at_ms += 1;
    }
    file.sync_all().unwrap();
}

/// One append into a ring already at the default cap: the line just added
/// is what the eviction loop drops, so the ring stays at the cap and every
/// iteration pays the full read-rewrite-`fsync`-rename-`fsync`.
fn bench_full_ring(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("barks.jsonl");
    seed_ring(&path, DEFAULT_MAX_BYTES);
    let new_bark = bark_for(u64::MAX);
    c.bench_function("bark_append/full_ring", |b| {
        b.iter(|| {
            barks::append(black_box(&path), black_box(&new_bark), black_box(DEFAULT_MAX_BYTES))
                .unwrap();
        });
    });
}

/// The same append into a fresh file: what the two `fsync`s and the rename
/// cost on their own, with no ring to read or rewrite.
fn bench_empty_ring(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("barks.jsonl");
    let new_bark = bark_for(0);
    c.bench_function("bark_append/empty_ring", |b| {
        b.iter_with_setup(
            // Untimed: each measured iteration has to start from no file,
            // and `append` treats a missing one as an empty ring.
            || {
                let _ = std::fs::remove_file(&path);
            },
            |_| {
                barks::append(&path, black_box(&new_bark), black_box(DEFAULT_MAX_BYTES)).unwrap();
            },
        );
    });
}

criterion_group!(benches, bench_full_ring, bench_empty_ring);
criterion_main!(benches);
