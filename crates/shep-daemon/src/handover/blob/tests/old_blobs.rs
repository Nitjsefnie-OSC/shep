//! Blobs committed as the bytes a named release wrote.
//!
//! Every file under `crates/shep-daemon/fixtures/handover/` is the output of
//! that release's own serialiser, run against its own `sample_handover()`,
//! copied here unchanged. Nothing regenerates them from this daemon's
//! [`Handover`]: a fixture built here would follow a field rename the way
//! [`compat`](super::compat)'s cases do, and the whole point of these is that
//! they cannot. `crates/shep-daemon/fixtures/handover/README.md` names which
//! tag wrote which file, and how it was dumped.
//!
//! Each test loads one file through [`Handover::load_value`], asserts the
//! shape that release wrote at the levels of the blob it changed, the values
//! that shape wrote, and that every key the release predates reads as `None`
//! through the accessor that reads it.

use std::os::fd::RawFd;
use std::path::PathBuf;

use shep_core::status::ProcStatus;

use crate::entry::ReloadState;
use crate::handover::{Counters, Handover};
use crate::privilege::SpawnIdentity;

/// The oldest blob in the set: the first release that handed a flock over at
/// all. Four descriptors per sheep, and no marker of any kind.
const V0_1_18: &str = "shep-daemon-v0.1.18";
/// Added `stdin` and `channel` to a sheep's descriptors.
const V0_1_20: &str = "shep-daemon-v0.1.20";
/// Added `pending_delete`, `manual`, `reload`, `ready_failed` and `reloads`.
const V0_1_21: &str = "shep-daemon-v0.1.21";
/// Added `restart_due`.
const V0_1_22: &str = "shep-daemon-v0.1.22";
/// Added `dog`.
const V0_1_23: &str = "shep-daemon-v0.1.23";
/// Added `pending` and `pending_reidentifies`.
const V0_1_30: &str = "shep-daemon-v0.1.30";
/// Added `depends_on` to the carried `AppConfig`.
const V0_5_0: &str = "shep-daemon-v0.5.0";
/// Added `environment` to the carried `AppConfig`.
const V0_6_2: &str = "shep-daemon-v0.6.2";
/// Added `level_rules` to the carried `AppConfig`, and dropped `increment_var`.
const V0_8_0: &str = "shep-daemon-v0.8.0";
/// Added `dogs` to the carried `AppConfig`.
const V0_11_0: &str = "shep-daemon-v0.11.0";
/// Added `log_timestamps` to the carried `AppConfig`.
const V0_12_0: &str = "shep-daemon-v0.12.0";
/// Added `disabled`.
const V0_13_0: &str = "shep-daemon-v0.13.0";

/// Every fixture, oldest first. Each entry is the first `shep-daemon-v*` tag
/// that wrote the shape its file holds.
const WRITERS: [&str; 12] = [
    V0_1_18, V0_1_20, V0_1_21, V0_1_22, V0_1_23, V0_1_30, V0_5_0, V0_6_2, V0_8_0, V0_11_0, V0_12_0,
    V0_13_0,
];

/// The keys every blob in this set has at the top level, from the first
/// release to the one before `reloads`.
const BLOB_BEFORE_RELOADS: [&str; 7] = [
    "listener_fd",
    "next_action_stamp",
    "next_deadline",
    "next_id",
    "pidfile_fd",
    "sheep",
    "version",
];

/// The keys a carried sheep has from the release that wrote no marker at all.
const SHEEP_BEFORE_MARKERS: [&str; 11] = [
    "app",
    "credentials",
    "epoch",
    "fds",
    "id",
    "instance",
    "last_exit",
    "name",
    "pid",
    "restarts",
    "status",
];

/// The four descriptors a sheep's output travelled on before `stdin`.
const FOUR_DESCRIPTORS: [&str; 4] = ["err_log", "err_pipe", "out_log", "out_pipe"];

/// The six, from the release that carried a stdin pipe and a channel.
const SIX_DESCRIPTORS: [&str; 6] = [
    "channel", "err_log", "err_pipe", "out_log", "out_pipe", "stdin",
];

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/handover")
}

/// Where a release's committed bytes live.
fn fixture_path(tag: &str) -> PathBuf {
    fixtures_dir().join(format!("{tag}.json"))
}

/// A fixture as raw JSON, so a test can name the keys it holds rather than
/// only the fields this image has a place for.
fn fixture_value(tag: &str) -> serde_json::Value {
    let path = fixture_path(tag);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("the fixture {} is not readable: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("the fixture {} is not JSON: {error}", path.display()))
}

/// The keys one JSON object holds, sorted: the shape its writer painted at
/// that level of the blob.
fn keys(object: &serde_json::Value) -> Vec<String> {
    let mut found: Vec<String> = object
        .as_object()
        .expect("a blob, a carried sheep and a carried sheep's descriptors are all objects")
        .keys()
        .cloned()
        .collect();
    found.sort_unstable();
    found
}

/// A literal list of key names, sorted the way [`keys`] sorts a real one.
fn sorted<T: AsRef<str>>(names: &[T]) -> Vec<String> {
    let mut found: Vec<String> = names.iter().map(|key| key.as_ref().to_owned()).collect();
    found.sort_unstable();
    found
}

/// `base`'s keys plus the ones `added` names, sorted: the shape a later
/// release wrote, read off the earlier shape this file already pins.
fn sorted_plus<T: AsRef<str>>(base: &[T], added: &[&str]) -> Vec<String> {
    let mut found = sorted(base);
    found.extend(added.iter().map(|key| (*key).to_owned()));
    found.sort_unstable();
    found
}

/// What every one of these blobs says, unchanged from the first to the last:
/// the plain `web` sheep `entry_fixture` builds, the daemon's two descriptor
/// numbers, and the three counters a successor must not reissue.
fn assert_the_plain_flock(blob: &Handover) {
    assert_eq!(
        blob.version, 1,
        "every release in this set wrote format version 1"
    );
    assert_eq!(blob.sheep.len(), 1, "one sheep: the fixture's `web` app");
    let sheep = &blob.sheep[0];
    assert_eq!(sheep.id(), 1);
    assert_eq!(sheep.name(), "web");
    assert_eq!(sheep.instance(), 0);
    assert_eq!(sheep.pid(), Some(100));
    assert_eq!(sheep.restarts(), 0);
    assert_eq!(sheep.epoch(), 7);
    assert_eq!(sheep.status(), ProcStatus::Online);
    assert_eq!(sheep.last_exit(), None);
    assert_eq!(sheep.credentials(), SpawnIdentity::Resolved(None));
    assert_eq!(sheep.app().name, "web");
    assert_eq!(sheep.app().script, "./srv");
    assert_eq!(
        blob.counters(),
        Counters {
            next_id: 9,
            next_deadline: 5,
            next_action_stamp: 2,
        }
    );
}

/// The four descriptors every release in this set carried, plus the two a
/// release from `stdin` onwards carried beside them.
fn assert_the_descriptors(blob: &Handover, stdin: Option<RawFd>, channel: Option<RawFd>) {
    let fds = blob.sheep[0].fds();
    assert_eq!(fds.out_pipe, Some(11));
    assert_eq!(fds.err_pipe, Some(12));
    assert_eq!(fds.out_log, Some(13));
    assert_eq!(fds.err_log, Some(14));
    assert_eq!(fds.stdin, stdin);
    assert_eq!(fds.channel, channel);
}

/// Every marker a pre-v0.1.21 release never wrote, read the way a successor
/// reads it: through the accessor, as `None`.
fn assert_nothing_marked(blob: &Handover) {
    let sheep = &blob.sheep[0];
    assert_eq!(sheep.pending_delete(), None);
    assert_eq!(sheep.manual(), None);
    assert_eq!(sheep.reload(), None);
    assert_eq!(sheep.ready_failed(), None);
    assert_eq!(sheep.restart_due(), None);
    assert_eq!(sheep.dog(), None);
    assert_eq!(sheep.pending(), None);
    assert_eq!(sheep.pending_reidentifies(), None);
    assert_eq!(sheep.disabled(), None);
    assert!(blob.reloads().is_empty());
}

/// The five keys v0.1.21 added, each carrying its own "nothing is happening
/// to this sheep". A successor that read them as absent instead of as these
/// would respawn a drainee into its own slot and forget a delete was asked for.
fn assert_the_plain_markers(blob: &Handover) {
    let sheep = &blob.sheep[0];
    assert_eq!(sheep.pending_delete(), Some(false));
    assert_eq!(
        sheep.manual(),
        None,
        "the key is there and says no command owns the exit"
    );
    assert_eq!(sheep.reload(), Some(ReloadState::None));
    assert_eq!(sheep.ready_failed(), Some(false));
    assert!(
        blob.reloads().is_empty(),
        "`reloads` is there and empty, which is the same answer as absent"
    );
}

#[test]
fn a_blob_from_shep_daemon_v0_1_18_still_loads() {
    let raw = fixture_value(V0_1_18);
    let blob = Handover::load_value(raw.clone())
        .expect("the oldest blob on disk must still load, or a successor refuses a flock");
    assert_the_plain_flock(&blob);

    assert_eq!(keys(&raw), sorted(&BLOB_BEFORE_RELOADS));
    assert_eq!(keys(&raw["sheep"][0]), sorted(&SHEEP_BEFORE_MARKERS));
    assert_eq!(keys(&raw["sheep"][0]["fds"]), sorted(&FOUR_DESCRIPTORS));

    assert_the_descriptors(&blob, None, None);
    assert_nothing_marked(&blob);

    // The config this release carried, and the keys later releases added to
    // it. `increment_var` is one this image has no field for at all: an
    // unknown key is dropped, not refused.
    let app = &raw["sheep"][0]["app"];
    assert!(app.get("increment_var").is_some());
    for added in [
        "depends_on",
        "environment",
        "level_rules",
        "dogs",
        "log_timestamps",
    ] {
        assert!(app.get(added).is_none(), "{added} is a later release's key");
    }
    assert!(blob.sheep[0].app().depends_on.is_empty());
    assert_eq!(blob.sheep[0].app().environment, None);
    assert!(blob.sheep[0].app().level_rules.is_empty());
    assert!(blob.sheep[0].app().dogs.is_empty());
}

#[test]
fn a_blob_from_shep_daemon_v0_1_20_still_loads() {
    let raw = fixture_value(V0_1_20);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(keys(&raw), sorted(&BLOB_BEFORE_RELOADS));
    assert_eq!(keys(&raw["sheep"][0]), sorted(&SHEEP_BEFORE_MARKERS));
    assert_eq!(keys(&raw["sheep"][0]["fds"]), sorted(&SIX_DESCRIPTORS));

    // The two descriptors this release added, carried whole.
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_nothing_marked(&blob);
}

#[test]
fn a_blob_from_shep_daemon_v0_1_21_still_loads() {
    let raw = fixture_value(V0_1_21);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(keys(&raw), sorted_plus(&BLOB_BEFORE_RELOADS, &["reloads"]));
    assert_eq!(
        keys(&raw["sheep"][0]),
        sorted_plus(
            &SHEEP_BEFORE_MARKERS,
            &["pending_delete", "manual", "reload", "ready_failed"]
        )
    );
    assert_eq!(keys(&raw["sheep"][0]["fds"]), sorted(&SIX_DESCRIPTORS));

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));

    // What it predates.
    let sheep = &blob.sheep[0];
    assert_eq!(sheep.restart_due(), None);
    assert_eq!(sheep.dog(), None);
    assert_eq!(sheep.pending(), None);
    assert_eq!(sheep.pending_reidentifies(), None);
    assert_eq!(sheep.disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_1_22_still_loads() {
    let raw = fixture_value(V0_1_22);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        sorted_plus(&keys(&fixture_value(V0_1_21)["sheep"][0]), &["restart_due"])
    );

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));

    // The key this release added, carrying that an `Online` sheep is owed no
    // respawn.
    let sheep = &blob.sheep[0];
    assert_eq!(sheep.restart_due(), None);
    assert_eq!(sheep.dog(), None);
    assert_eq!(sheep.pending(), None);
    assert_eq!(sheep.pending_reidentifies(), None);
    assert_eq!(sheep.disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_1_23_still_loads() {
    let raw = fixture_value(V0_1_23);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        sorted_plus(&keys(&fixture_value(V0_1_22)["sheep"][0]), &["dog"])
    );

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));

    let sheep = &blob.sheep[0];
    assert_eq!(sheep.restart_due(), None);
    // The marker this release added: an ordinary sheep, not a dog.
    assert_eq!(sheep.dog(), None);
    assert_eq!(sheep.pending(), None);
    assert_eq!(sheep.pending_reidentifies(), None);
    assert_eq!(sheep.disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_1_30_still_loads() {
    let raw = fixture_value(V0_1_30);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        sorted_plus(
            &keys(&fixture_value(V0_1_23)["sheep"][0]),
            &["pending", "pending_reidentifies"]
        )
    );

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));

    let sheep = &blob.sheep[0];
    assert_eq!(sheep.restart_due(), None);
    assert_eq!(sheep.dog(), None);
    // The pair this release added, written together and here both empty:
    // nothing is parked for this sheep.
    assert_eq!(sheep.pending(), None);
    assert_eq!(sheep.pending_reidentifies(), None);
    assert_eq!(sheep.disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_5_0_still_loads() {
    let raw = fixture_value(V0_5_0);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        keys(&fixture_value(V0_1_30)["sheep"][0])
    );

    // The key this release added to the config a sheep carries.
    assert_eq!(raw["sheep"][0]["app"]["depends_on"], serde_json::json!([]));
    assert!(blob.sheep[0].app().depends_on.is_empty());

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_eq!(blob.sheep[0].dog(), None);
    assert_eq!(blob.sheep[0].pending(), None);
    assert_eq!(blob.sheep[0].disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_6_2_still_loads() {
    let raw = fixture_value(V0_6_2);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        keys(&fixture_value(V0_1_30)["sheep"][0])
    );

    // The key this release added, carrying no environment name.
    assert_eq!(
        raw["sheep"][0]["app"]["environment"],
        serde_json::Value::Null
    );
    assert_eq!(blob.sheep[0].app().environment, None);

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_eq!(blob.sheep[0].pending(), None);
    assert_eq!(blob.sheep[0].disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_8_0_still_loads() {
    let raw = fixture_value(V0_8_0);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        keys(&fixture_value(V0_1_30)["sheep"][0])
    );

    // The key this release added, and the one it dropped. A blob from the
    // release before still carries `increment_var`, which is what the v0.1.18
    // fixture holds.
    let app = &raw["sheep"][0]["app"];
    assert_eq!(app["level_rules"], serde_json::json!([]));
    assert!(app.get("increment_var").is_none());
    assert!(blob.sheep[0].app().level_rules.is_empty());

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_eq!(blob.sheep[0].pending(), None);
    assert_eq!(blob.sheep[0].disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_11_0_still_loads() {
    let raw = fixture_value(V0_11_0);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        keys(&fixture_value(V0_1_30)["sheep"][0])
    );

    // The key this release added: an app with no dog table.
    assert_eq!(raw["sheep"][0]["app"]["dogs"], serde_json::json!({}));
    assert!(blob.sheep[0].app().dogs.is_empty());

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_eq!(blob.sheep[0].dog(), None);
    assert_eq!(blob.sheep[0].disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_12_0_still_loads() {
    let raw = fixture_value(V0_12_0);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        keys(&fixture_value(V0_1_30)["sheep"][0])
    );

    // The key this release added, carrying the stamp-on default it wrote.
    assert_eq!(
        raw["sheep"][0]["app"]["log_timestamps"],
        serde_json::json!(true)
    );
    assert!(blob.sheep[0].app().log_timestamps);

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_eq!(blob.sheep[0].pending(), None);
    assert_eq!(blob.sheep[0].disabled(), None);
}

#[test]
fn a_blob_from_shep_daemon_v0_13_0_still_loads() {
    let raw = fixture_value(V0_13_0);
    let blob = Handover::load_value(raw.clone()).expect("an older blob must still load");
    assert_the_plain_flock(&blob);

    assert_eq!(
        keys(&raw["sheep"][0]),
        sorted_plus(&keys(&fixture_value(V0_1_30)["sheep"][0]), &["disabled"])
    );

    // The key this release added, carrying that nothing is disabled.
    assert_eq!(blob.sheep[0].disabled(), None);

    assert_the_plain_markers(&blob);
    assert_the_descriptors(&blob, Some(15), Some(16));
    assert_eq!(blob.sheep[0].pending(), None);
}

/// The legacy arm of the boot: a predecessor from before the envelope left
/// its blob at `$SHEP_HOME/run/handover.json`, and
/// read it through [`Handover::read`]. The oldest fixture is the blob that
/// path is for.
#[test]
fn the_oldest_blob_loads_through_the_legacy_read_path() {
    let dir = tempfile::tempdir().expect("a temp home for the legacy blob");
    let written = dir.path().join("handover.json");
    std::fs::write(
        &written,
        std::fs::read(fixture_path(V0_1_18)).expect("the fixture is readable"),
    )
    .expect("the legacy blob is writable");

    let blob = Handover::read(&written).expect("the legacy arm reads what v0.1.18 wrote");
    assert_the_plain_flock(&blob);
    assert_the_descriptors(&blob, None, None);
    assert_nothing_marked(&blob);
}

/// The set of fixtures and the set of tags this file tests are one set. A file
/// with no test is an old blob nothing checks; a test naming a file that is
/// not there is a claim about bytes that do not exist.
#[test]
fn every_fixture_has_a_test_and_every_test_has_a_fixture() {
    let mut on_disk: Vec<String> = std::fs::read_dir(fixtures_dir())
        .expect("the fixture directory is readable")
        .map(|entry| entry.expect("a fixture entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .map(|path| {
            path.file_stem()
                .expect("a fixture is named")
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    on_disk.sort();

    let mut named: Vec<String> = WRITERS.iter().map(|tag| (*tag).to_owned()).collect();
    named.sort();

    assert_eq!(
        on_disk, named,
        "fixtures/handover and this file's list of tags have drifted apart"
    );
}

/// A fixture is only worth what its bytes are, and those bytes are sample
/// data. A home path or a hostname from the machine that dumped one is a
/// record of that machine, not of the release.
#[test]
fn no_fixture_carries_the_machine_that_dumped_it() {
    for tag in WRITERS {
        let text = std::fs::read_to_string(fixture_path(tag)).expect("the fixture is readable");
        assert!(
            !text.contains("/Users") && !text.contains("/home"),
            "{tag} carries a home path from the machine that dumped it"
        );
        assert!(
            text.contains("\"name\":\"web\""),
            "{tag} is not the `web` app the handover fixtures build"
        );
    }
}
