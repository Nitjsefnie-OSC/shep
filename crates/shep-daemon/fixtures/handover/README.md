# Handover blob fixtures

Committed bytes of the handover blob each `shep-daemon` release wrote, so a
successor's ability to read an older blob is checked against bytes that release
produced rather than against this daemon's own serialiser.

`crates/shep-daemon/src/handover/blob/tests/old_blobs.rs` and
`crates/shep-daemon/src/handover/blob/tests/old_blob_state.rs` load every file
here. Nothing in this repository regenerates them, and nothing should: a
fixture written from the current `Handover` follows a field rename, which is
the failure these exist to catch.

## Policy

One file per distinct shape that shipped, named for the first `shep-daemon-v*`
tag that wrote it. `VERSION` in `crates/shep-daemon/src/handover/blob.rs` has
stayed 1 since the handover existed, so the release tag is the only key there
is. A shape is the set of keys a serialised blob holds, at the blob, at
`sheep[]`, at `sheep[].fds` and at `reloads`.

Each of those files also has a `<tag>-populated.json` sibling: the same
release's blob with every optional field that release could carry filled in.
A shape is a set of keys, and a key set alone cannot catch a rename: an absent
key and a `null` read the same, and `AppConfig` carries `#[serde(default)]`, so
a renamed config field hands the successor that field's own default. The
populated sibling is what makes each carried value an assertion.

The hunt: every tag from `shep-daemon-v0.1.18` (the first with a handover) to
`shep-daemon-v0.14.0`, 61 tags, each dumped from its own code. 12 distinct key
sets came out. `shep-daemon`'s own structs account for seven of them and the
`AppConfig` a sheep carries accounts for the other five.

## How each file was made

For each tag below: a throwaway tree at that tag outside this checkout
(`git archive <tag> | tar -x`), one throwaway test added to it that prints
`serde_json::to_string(&sample_handover())` from that release's own test
helpers, and this run to capture it:

```bash
cargo test -p shep-daemon --lib dump_blob_for_fixture_bytes -- --nocapture
```

`sample_handover()` is that release's own: the plain `web` app its
`entry_fixture` builds, `fds_at(11)`, epoch 7, listener fd 3, pidfile fd 4, and
the counters 9, 5, 2. Sample data only. No real flock, no home path, no
hostname. The output was copied here unchanged, one line of JSON per file.

For each `-populated` sibling, the same tree with a second throwaway test that
fills in every optional key that release's own blob had a place for, then
round-trips the result through that release's `Handover` before printing it:

```bash
cargo test -p shep-daemon --lib dump_state_blob_for_fixture_bytes -- --nocapture
```

The round trip is the point. The release's own deserialiser had to accept the
filled-in blob for its own serialiser to have written it, so these are blobs
that release could really have left on disk, not today's types dressed in old
key names. Each carries three sheep: an `Online` sheep with a parked config and
a delete asked for, a `Stopping` drainee mid-swap with its reload job, and a
`waiting-restart` sheep owed a respawn with its deadline. Keys a release had no
place for stay absent, so v0.1.18 and v0.1.20 populate the config only.

## The files

`Crosses as` is how that release moved the blob: `file` is
`$SHEP_HOME/run/handover.json`, read back through `Handover::read`; `envelope`
is the state of a `shep-handover` envelope, from `shep-daemon-v0.13.0`, the
first release that used the crate.

| File | Tag | Crosses as | What the shape added |
| --- | --- | --- | --- |
| `shep-daemon-v0.1.18.json` | `shep-daemon-v0.1.18` | file | the blob itself: four descriptors per sheep, no marker |
| `shep-daemon-v0.1.20.json` | `shep-daemon-v0.1.20` | file | `stdin` and `channel` in `sheep[].fds` |
| `shep-daemon-v0.1.21.json` | `shep-daemon-v0.1.21` | file | `reloads`, `pending_delete`, `manual`, `reload`, `ready_failed` |
| `shep-daemon-v0.1.22.json` | `shep-daemon-v0.1.22` | file | `restart_due` |
| `shep-daemon-v0.1.23.json` | `shep-daemon-v0.1.23` | file | `dog` |
| `shep-daemon-v0.1.30.json` | `shep-daemon-v0.1.30` | file | `pending` and `pending_reidentifies` |
| `shep-daemon-v0.5.0.json` | `shep-daemon-v0.5.0` | file | `depends_on` in `sheep[].app` |
| `shep-daemon-v0.6.2.json` | `shep-daemon-v0.6.2` | file | `environment` in `sheep[].app` |
| `shep-daemon-v0.8.0.json` | `shep-daemon-v0.8.0` | file | `level_rules` in `sheep[].app`, and dropped `increment_var` |
| `shep-daemon-v0.11.0.json` | `shep-daemon-v0.11.0` | file | `dogs` in `sheep[].app` |
| `shep-daemon-v0.12.0.json` | `shep-daemon-v0.12.0` | file | `log_timestamps` in `sheep[].app` |
| `shep-daemon-v0.13.0.json` | `shep-daemon-v0.13.0` | envelope | `disabled` |

Two shapes are worth naming because they are the ones a reader would not guess:

- `increment_var` is in every blob from v0.1.18 through v0.7.4 and has no
  field in today's `AppConfig`. Eight of these twelve fixtures hold it, as
  `null`: v0.1.18, v0.1.20, v0.1.21, v0.1.22, v0.1.23, v0.1.30, v0.5.0 and
  v0.6.2. `old_blobs.rs` checks that list against the bytes rather than
  trusting it.
- `shep-daemon-v0.8.0` is the first blob without it, so it is a distinct shape
  rather than v0.7.4's with one key missing.

Each tag in the table has a `<tag>-populated.json` sibling, so 24 files in
total. The table names the shape once rather than twice: the sibling holds the
same keys with values in them.
