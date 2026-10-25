//! Shared test fixtures: the `ProcessInfo` and `BleatsArgs` builders, the
//! log-file helper, and [`run_bleats`], the harness every scripted run in
//! [`follow`] and [`tail`] goes through. [`follow`] covers the
//! bus-subscribed path, [`tail`] the `--no-follow` file-reading one,
//! [`remote`] the tail read through a host's shepherd and [`host_follow`]
//! how a run over a host ends.

use std::time::Duration;

use shep_client::testing::fake_client_with_push;
use shep_core::protocol::ProcessInfo;
use shep_core::status::ProcStatus;

use super::*;
use crate::cli::{Cli, Commands};

/// The row [`info`] and [`info_with_instance`] hand back: an online sheep
/// with a pid and both log paths. `slot: None` is a sheep the shepherd
/// registered with no instance slot, and it is listed with the `-0-` paths
/// a single-instance app's row carries.
fn process_row(id: u32, name: &str, slot: Option<u32>) -> ProcessInfo {
    let path_slot = slot.unwrap_or(0);
    ProcessInfo::builder(id, name, ProcStatus::Online)
        .pid(Some(1000 + id))
        .instance(slot)
        .out_file(Some(format!("/logs/{name}-{path_slot}-out.log")))
        .err_file(Some(format!("/logs/{name}-{path_slot}-err.log")))
        .build()
}

fn info(id: u32, name: &str) -> ProcessInfo {
    process_row(id, name, None)
}

/// Like [`info`], but with a real instance slot: `info` never sets one.
fn info_with_instance(id: u32, name: &str, slot: u32) -> ProcessInfo {
    process_row(id, name, Some(slot))
}

fn bleats_args(selector: &str, no_follow: bool, err: bool, out: bool) -> BleatsArgs {
    BleatsArgs {
        selector: selector.to_string(),
        no_follow,
        // The follow tests here assert on what the bus delivers, so
        // this asks for no history.
        lines: crate::cli::DEFAULT_BLEAT_LINES,
        err,
        out,
    }
}

fn follow_args(selector: &str) -> BleatsArgs {
    BleatsArgs {
        lines: 0,
        ..bleats_args(selector, false, false, false)
    }
}

fn follow_args_err(selector: &str) -> BleatsArgs {
    bleats_args(selector, false, true, false)
}

fn follow_args_out(selector: &str) -> BleatsArgs {
    bleats_args(selector, false, false, true)
}

fn no_follow_args(selector: &str) -> BleatsArgs {
    bleats_args(selector, true, false, false)
}

fn no_follow_args_err(selector: &str) -> BleatsArgs {
    bleats_args(selector, true, true, false)
}

fn no_follow_args_out(selector: &str) -> BleatsArgs {
    bleats_args(selector, true, false, true)
}

/// Writes `content` to `dir/name` and returns the path as a `String`,
/// what a scripted [`ProcessInfo`]'s `out_file`/`err_file` needs.
fn write_log(dir: &Path, name: &str, content: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path.to_str().unwrap().to_string()
}

#[test]
fn no_follow_parses_and_plain_bleats_still_follows() {
    use clap::Parser;

    let Commands::Bleats(args) = Cli::try_parse_from(["shep", "bleats"]).unwrap().command else {
        panic!()
    };
    assert!(!args.no_follow, "the default is to follow");

    let Commands::Bleats(args) = Cli::try_parse_from(["shep", "bleats", "--no-follow"])
        .unwrap()
        .command
    else {
        panic!()
    };
    assert!(args.no_follow);

    // `--no-follow` is `ArgAction::SetTrue` and stores no value, so a
    // following token is not consumed by it and lands on the positional.
    let Commands::Bleats(args) = Cli::try_parse_from(["shep", "bleats", "--no-follow", "true"])
        .unwrap()
        .command
    else {
        panic!()
    };
    assert!(args.no_follow);
    assert_eq!(
        args.selector, "true",
        "--no-follow takes no value; the token is the selector"
    );
}

/// Every `bleats(...)`/`bleats_with_signal(...)` call in this module is
/// bounded by this timeout, so a hang fails with a named assertion
/// instead of a killed CI job.
const RUN_TIMEOUT: Duration = Duration::from_secs(5);

/// The harness every scripted run here goes through: [`bleats`] against
/// `client` into buffers this owns, bounded by [`RUN_TIMEOUT`], and the
/// exit code beside what the verb wrote to each stream.
///
/// Setup only, never an assertion: what a test asserts stays in the test.
/// No `#[track_caller]`: rustc does not carry a caller location through an
/// `async fn` (rust-lang/rust#110011), so it would only warn.
async fn run_bleats(client: &Client, args: &BleatsArgs, fmt: Format) -> (ExitCode, String, String) {
    run_bleats_quiet(client, args, fmt, false).await
}

/// [`run_bleats`] with `quiet` given, which the tests that assert on what
/// `quiet` suppresses need.
async fn run_bleats_quiet(
    client: &Client,
    args: &BleatsArgs,
    fmt: Format,
    quiet: bool,
) -> (ExitCode, String, String) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = {
        let mut streams = Streams::bare(&mut out, &mut err, fmt);
        tokio::time::timeout(RUN_TIMEOUT, bleats(client, &mut streams, quiet, args))
            .await
            .expect("the scripted run ends on its own, not by hanging")
    };
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

mod colour;
mod follow;
mod host_follow;
mod remote;
mod tail;
