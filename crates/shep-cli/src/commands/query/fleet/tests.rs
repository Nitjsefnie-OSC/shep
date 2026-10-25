use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use shep_client::SshOptions;
use shep_core::paths::ShepPaths;
use shep_core::protocol::{
    Envelope, HelloAck, HelloReply, PROTOCOL_VERSION, ProcessInfo, Reply, Request, Response,
    ServerFrame, encode_frame,
};
use shep_core::status::ProcStatus;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::cli::FlockArgs;
use crate::commands::query::testing::sample_host;
use crate::remote::Link;
use crate::version_guard::flock_command_on;

/// Room for a stand-in to start and answer, short enough that a hang fails
/// the test quickly.
const ROOMY: Duration = Duration::from_secs(10);

fn paths_in(dir: &Path) -> ShepPaths {
    let paths = ShepPaths::resolve(&|_| None, dir);
    std::fs::create_dir_all(&paths.run).unwrap();
    paths
}

fn sheep(name: &str) -> ProcessInfo {
    ProcessInfo::builder(1, name, ProcStatus::Online).build()
}

/// A shepherd of `version` at `paths.socket` listing one sheep, `name`. The
/// receiver must be held: the fake stops serving once nothing reads it.
async fn local_shepherd(
    paths: &ShepPaths,
    version: &str,
    name: &str,
) -> UnboundedReceiver<Envelope> {
    let ack = HelloAck {
        daemon_version: version.to_owned(),
        protocol: PROTOCOL_VERSION,
        pid: 4242,
        min_supported: None,
    };
    let info = sheep(name);
    shep_client::testing::fake_daemon_answering_with_ack(&paths.socket, ack, move |request| {
        match request {
            Request::ListFlock => Response::Flock(vec![info.clone()]),
            Request::HostUsage => Response::HostUsage(Some(sample_host())),
            _ => Response::Pong,
        }
    })
    .await
}

/// `payload` behind the four-byte length every frame carries.
fn framed(payload: &impl serde::Serialize) -> Vec<u8> {
    let payload = encode_frame(payload).unwrap();
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend_from_slice(&payload);
    bytes
}

/// The one `ssh` every host shares: it records its argv as `<host>.argv`,
/// then runs `<host>.sh`, which a test writes.
///
/// `serve`, for a script to call, answers the way a shepherd does: it reads
/// a frame and replies, three times over (the handshake, then `ListFlock` as
/// request 1 and `HostUsage` as request 2).
fn write_ssh(dir: &Path) {
    let program = dir.join("ssh");
    std::fs::write(
        &program,
        r#"#!/bin/sh
dir=$(dirname "$0")
dest=; prev=
for a in "$@"; do [ "$prev" = -- ] && dest=$a; prev=$a; done
for a in "$@"; do echo "$a"; done > "$dir/$dest.argv"
skip() {
  n=0
  for b in $(dd bs=1 count=4 2>/dev/null | od -An -v -tu1); do n=$((n * 256 + b)); done
  [ "$n" -eq 0 ] || dd bs=1 count="$n" of=/dev/null 2>/dev/null
}
serve() {
  skip; cat "$dir/$dest.hello"
  skip; cat "$dir/$dest.list"
  skip; cat "$dir/$dest.usage"
  cat >/dev/null
}
. "$dir/$dest.sh"
"#,
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Makes `host` run `script` when it is dialled.
fn host_runs(dir: &Path, host: &str, script: &str) {
    std::fs::write(dir.join(format!("{host}.sh")), script).unwrap();
}

/// Makes `host` a shepherd of `version` listing one sheep, `name`.
fn host_answers_as(dir: &Path, host: &str, version: &str, name: &str) {
    host_speaks(dir, host, version, PROTOCOL_VERSION, name);
}

/// Makes `host` a shepherd of `version` speaking `protocol`, listing one
/// sheep, `name`.
fn host_speaks(dir: &Path, host: &str, version: &str, protocol: u32, name: &str) {
    let ack: HelloReply = Ok(HelloAck {
        daemon_version: version.to_owned(),
        protocol,
        pid: 4242,
        min_supported: None,
    });
    let reply = |id, response| {
        framed(&ServerFrame::Reply(Reply {
            id,
            result: Ok(response),
        }))
    };
    for (extension, bytes) in [
        ("hello", framed(&ack)),
        ("list", reply(1, Response::Flock(vec![sheep(name)]))),
        ("usage", reply(2, Response::HostUsage(Some(sample_host())))),
    ] {
        std::fs::write(dir.join(format!("{host}.{extension}")), bytes).unwrap();
    }
    host_runs(dir, host, "serve");
}

/// Makes `host` a shepherd of this build listing one sheep, `name`.
fn host_answers(dir: &Path, host: &str, name: &str) {
    host_answers_as(dir, host, env!("CARGO_PKG_VERSION"), name);
}

/// Makes `host` an ssh that cannot log in.
fn host_refuses(dir: &Path, host: &str) {
    host_runs(
        dir,
        host,
        &format!("echo '{host}: Permission denied (publickey).' >&2\nexit 255"),
    );
}

struct Run {
    code: ExitCode,
    out: String,
    err: String,
}

/// `fleet` over `hosts`, dialled through the stand-in in `dir`.
async fn run_with(dir: &Path, hosts: &[&str], timeout: Duration) -> Run {
    run_guarded(dir, hosts, timeout, VersionGuard::Warn).await
}

/// [`run_with`] under `guard`.
async fn run_guarded(dir: &Path, hosts: &[&str], timeout: Duration, guard: VersionGuard) -> Run {
    run_formatted(dir, hosts, timeout, guard, Format::Table).await
}

/// [`run_guarded`] in `fmt`.
async fn run_formatted(
    dir: &Path,
    hosts: &[&str],
    timeout: Duration,
    guard: VersionGuard,
    fmt: Format,
) -> Run {
    let paths = paths_in(dir);
    let ssh = unattended(timeout).program(dir.join("ssh"));
    let hosts: Vec<String> = hosts.iter().map(|&host| host.to_owned()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = fleet(
        &mut Streams::bare(&mut out, &mut err, fmt),
        &paths,
        guard,
        &hosts,
        &ssh,
        timeout,
    )
    .await;
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

async fn run(dir: &Path, hosts: &[&str]) -> Run {
    run_with(dir, hosts, ROOMY).await
}

/// A scratch directory holding the stand-in `ssh`, and this machine's
/// shepherd listing `local-sheep`.
async fn fixture() -> (tempfile::TempDir, UnboundedReceiver<Envelope>) {
    let dir = tempfile::tempdir().unwrap();
    write_ssh(dir.path());
    let held = local_shepherd(
        &paths_in(dir.path()),
        env!("CARGO_PKG_VERSION"),
        "local-sheep",
    )
    .await;
    (dir, held)
}

#[tokio::test]
async fn a_fleet_listing_prints_local_first_then_each_host_in_list_order_under_its_name() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp2", "amp2-sheep");
    host_answers(dir.path(), "amp1", "amp1-sheep");

    let run = run(dir.path(), &["amp2", "amp1"]).await;

    assert_eq!(run.code, ExitCode::Success, "{}", run.err);
    assert_eq!(run.err, "");
    let strip = "host  cpu  11%   mem  37.2G / 48.0G   disk  r 1.2M/s w 480.0K/s   net  rx 24.0K/s tx 9.0K/s";
    let expected = [
        "local",
        strip,
        "",
        "ID  NAME         STATUS  PID  RESTARTS  EXIT  CFG  CPU  MEM  UPTIME  FOLD  SMIT",
        "1   local-sheep  online  -    0         -     -    -    -    0s      -     -",
        "",
        "amp2",
        strip,
        "",
        "ID  NAME        STATUS  PID  RESTARTS  EXIT  CFG  CPU  MEM  UPTIME  FOLD  SMIT",
        "1   amp2-sheep  online  -    0         -     -    -    -    0s      -     -",
        "",
        "amp1",
        strip,
        "",
        "ID  NAME        STATUS  PID  RESTARTS  EXIT  CFG  CPU  MEM  UPTIME  FOLD  SMIT",
        "1   amp1-sheep  online  -    0         -     -    -    -    0s      -     -",
        "",
    ]
    .join("\n");
    assert_eq!(run.out, expected);
}

#[tokio::test]
async fn a_fleet_listing_as_json_is_one_envelope_in_list_order() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp2", "amp2-sheep");
    host_refuses(dir.path(), "amp3");
    host_answers(dir.path(), "amp1", "amp1-sheep");

    let run = run_formatted(
        dir.path(),
        &["amp2", "amp3", "amp1"],
        ROOMY,
        VersionGuard::Warn,
        Format::Json,
    )
    .await;

    assert_eq!(run.code, ExitCode::DaemonUnreachable);
    assert_eq!(run.out.lines().count(), 1, "one envelope: {}", run.out);
    let json: serde_json::Value = serde_json::from_str(&run.out).unwrap();
    assert_eq!(json["schema_version"], 1);
    let rows: Vec<(&str, &serde_json::Value)> = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["name"].as_str().unwrap(), &row["origin"]))
        .collect();
    assert_eq!(
        rows,
        [
            ("local-sheep", &serde_json::Value::Null),
            ("amp2-sheep", &serde_json::json!("amp2")),
            ("amp1-sheep", &serde_json::json!("amp1")),
        ]
    );
    assert_eq!(json["host"]["memory_total_bytes"], 51_539_607_552_u64);
    let hosts = json["hosts"].as_object().unwrap();
    assert_eq!(hosts.keys().collect::<Vec<_>>(), ["amp1", "amp2"]);
    assert_eq!(hosts["amp2"]["memory_total_bytes"], 51_539_607_552_u64);
    assert_eq!(run.err.lines().count(), 1, "{}", run.err);
    assert!(run.err.contains("amp3: "), "{}", run.err);
}

#[tokio::test]
async fn a_host_flag_listing_as_json_carries_origin_on_every_row() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp2", "amp2-sheep");
    let single = Link::Host {
        dest: "amp2",
        options: SshOptions::default().program(dir.path().join("ssh")),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut streams = Streams::bare(&mut out, &mut err, Format::Json);
    let args = FlockArgs {
        follow: false,
        interval: crate::cli::FOLLOW_INTERVAL_FLOOR_SECONDS,
    };

    let code = flock_command_on(&mut streams, &single, VersionGuard::Warn, &args, false).await;

    assert_eq!(code, ExitCode::Success, "{}", String::from_utf8_lossy(&err));
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["data"][0]["name"], "amp2-sheep");
    assert_eq!(json["data"][0]["origin"], "amp2");
    assert_eq!(json["host"]["memory_total_bytes"], 51_539_607_552_u64);
}

#[tokio::test]
async fn every_host_answering_exits_success() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp1", "amp1-sheep");

    let run = run(dir.path(), &["amp1"]).await;

    assert_eq!(run.code, ExitCode::Success);
    assert_eq!(run.err, "");
}

#[tokio::test]
async fn a_host_that_fails_is_named_on_stderr_and_the_answering_hosts_still_print() {
    let (dir, _held) = fixture().await;
    host_refuses(dir.path(), "amp1");
    host_answers(dir.path(), "amp2", "amp2-sheep");

    let run = run(dir.path(), &["amp1", "amp2"]).await;

    assert_eq!(run.code, ExitCode::DaemonUnreachable);
    assert!(run.out.starts_with("local\n"), "{}", run.out);
    assert!(run.out.contains("local-sheep"), "{}", run.out);
    assert!(run.out.contains("\namp2\n"), "{}", run.out);
    assert!(run.out.contains("amp2-sheep"), "{}", run.out);
    assert!(
        !run.out.contains("amp1"),
        "no table for a failed host: {}",
        run.out
    );
    assert_eq!(run.err.lines().count(), 1, "{}", run.err);
    assert!(run.err.starts_with("error["), "{}", run.err);
    assert!(run.err.contains("amp1: "), "{}", run.err);
    assert!(
        run.err.contains("Permission denied (publickey)."),
        "{}",
        run.err
    );
}

#[tokio::test]
async fn the_exit_code_is_the_first_failing_hosts_in_list_order() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp1", "amp1-sheep");
    host_refuses(dir.path(), "amp3");

    // `-bad` is not a destination (InvalidConfig); `amp3` cannot log in
    // (DaemonUnreachable). Whichever comes first in the list sets the code.
    let invalid_first = run(dir.path(), &["amp1", "-bad", "amp3"]).await;
    let unreachable_first = run(dir.path(), &["amp1", "amp3", "-bad"]).await;

    assert_eq!(invalid_first.code, ExitCode::InvalidConfig);
    assert_eq!(unreachable_first.code, ExitCode::DaemonUnreachable);
}

#[tokio::test]
async fn a_dead_local_shepherd_prints_its_roll_under_local_and_exits_daemon_unreachable_after_the_hosts_still_print()
 {
    let dir = tempfile::tempdir().unwrap();
    write_ssh(dir.path());
    let paths = paths_in(dir.path());
    let roll =
        shep_daemon::snapshot::FlockSnapshot::with_apps(vec![shep_daemon::snapshot::SavedApp {
            app: shep_core::config::AppConfig::minimal("rolled-sheep", "./srv"),
            instances_running: 1,
            disabled: false,
        }]);
    std::fs::write(&paths.snapshot, serde_json::to_vec(&roll).unwrap()).unwrap();
    host_answers(dir.path(), "amp1", "amp1-sheep");

    let run = run(dir.path(), &["amp1"]).await;

    assert_eq!(run.code, ExitCode::DaemonUnreachable);
    assert!(run.out.starts_with("local\n"), "{}", run.out);
    let (local, remote) = run.out.split_once("\namp1\n").expect(&run.out);
    assert!(local.contains("rolled-sheep"), "{local}");
    assert!(remote.contains("amp1-sheep"), "{remote}");
    assert!(run.err.contains("no shepherd running"), "{}", run.err);
}

#[tokio::test]
async fn a_host_that_never_answers_is_abandoned_at_the_timeout_without_stalling_the_rest() {
    let (dir, _held) = fixture().await;
    host_runs(dir.path(), "amp1", "exec sleep 30");
    host_answers(dir.path(), "amp2", "amp2-sheep");

    let started = Instant::now();
    let run = run_with(dir.path(), &["amp1", "amp2"], Duration::from_millis(500)).await;

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(run.code, ExitCode::DaemonUnreachable);
    assert!(run.out.contains("amp2-sheep"), "{}", run.out);
    assert!(run.err.contains("amp1: "), "{}", run.err);
}

#[tokio::test]
async fn hosts_are_dialled_in_parallel() {
    let (dir, _held) = fixture().await;
    for (host, other) in [("amp1", "amp2"), ("amp2", "amp1")] {
        host_answers(dir.path(), host, &format!("{host}-sheep"));
        // Each host announces itself and then waits to hear from the other,
        // so a dial that waits for the previous one to finish never ends.
        host_runs(
            dir.path(),
            host,
            &format!(
                "touch \"$dir/{host}.up\"\n\
                 while [ ! -f \"$dir/{other}.up\" ]; do sleep 0.02; done\n\
                 serve"
            ),
        );
    }

    let run = run_with(dir.path(), &["amp1", "amp2"], Duration::from_secs(5)).await;

    assert_eq!(run.code, ExitCode::Success, "{}", run.err);
    assert!(
        run.out.contains("amp1-sheep") && run.out.contains("amp2-sheep"),
        "{}",
        run.out
    );
}

#[tokio::test]
async fn fan_out_passes_batch_mode_and_a_single_host_does_not() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp1", "amp1-sheep");
    host_answers(dir.path(), "amp2", "amp2-sheep");

    run(dir.path(), &["amp1"]).await;
    let single = Link::Host {
        dest: "amp2",
        options: SshOptions::default().program(dir.path().join("ssh")),
    };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut streams = Streams::bare(&mut out, &mut err, Format::Table);
    let args = FlockArgs {
        follow: false,
        interval: crate::cli::FOLLOW_INTERVAL_FLOOR_SECONDS,
    };
    flock_command_on(&mut streams, &single, VersionGuard::Warn, &args, false).await;

    let argv =
        |host: &str| std::fs::read_to_string(dir.path().join(format!("{host}.argv"))).unwrap();
    assert!(
        argv("amp1").lines().any(|line| line == "BatchMode=yes"),
        "{}",
        argv("amp1")
    );
    assert!(!argv("amp2").contains("BatchMode"), "{}", argv("amp2"));
}

#[tokio::test]
async fn an_invalid_hosts_entry_fails_alone_and_is_never_passed_to_ssh() {
    let (dir, _held) = fixture().await;
    host_answers(dir.path(), "amp1", "amp1-sheep");

    let run = run(dir.path(), &["-oProxyCommand=touch pwned", "amp1"]).await;

    assert_eq!(run.code, ExitCode::InvalidConfig);
    assert!(
        run.err.contains("-oProxyCommand=touch pwned: "),
        "{}",
        run.err
    );
    assert!(run.out.contains("amp1-sheep"), "{}", run.out);
    let stand_in_calls: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".argv"))
        .collect();
    assert_eq!(stand_in_calls, ["amp1.argv"]);
}

#[tokio::test]
async fn a_lagging_host_is_listed_with_the_version_warning_and_does_not_change_the_exit_code() {
    let (dir, _held) = fixture().await;
    host_answers_as(dir.path(), "amp1", "0.0.1", "amp1-sheep");

    let run = run(dir.path(), &["amp1"]).await;

    assert_eq!(run.code, ExitCode::Success, "{}", run.err);
    assert!(run.out.contains("amp1-sheep"), "{}", run.out);
    assert!(
        run.err.contains("the shepherd on amp1 is 0.0.1"),
        "{}",
        run.err
    );
    assert!(
        run.err.contains("ssh amp1 shep daemon reload"),
        "{}",
        run.err
    );
}

#[tokio::test]
async fn a_local_shepherd_that_never_answers_is_abandoned_at_the_timeout_and_the_hosts_still_print()
{
    let dir = tempfile::tempdir().unwrap();
    write_ssh(dir.path());
    host_answers(dir.path(), "amp1", "amp1-sheep");
    let listener = tokio::net::UnixListener::bind(&paths_in(dir.path()).socket).unwrap();
    // Accepts and holds the connection open without a word, so the handshake
    // never completes.
    let held = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            open.push(stream);
        }
    });

    let started = Instant::now();
    let run = run_with(dir.path(), &["amp1"], Duration::from_millis(500)).await;
    held.abort();

    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(run.code, ExitCode::DaemonUnreachable);
    assert_eq!(run.out.lines().next(), Some("amp1"), "{}", run.out);
    assert!(run.out.contains("amp1-sheep"), "{}", run.out);
    assert!(
        run.err.contains("local: did not answer within 500ms"),
        "{}",
        run.err
    );
}

#[tokio::test]
async fn a_host_on_another_protocol_is_refused_when_the_guard_enforces_and_the_rest_still_print() {
    let (dir, _held) = fixture().await;
    host_speaks(
        dir.path(),
        "amp1",
        "0.0.1",
        PROTOCOL_VERSION - 1,
        "amp1-sheep",
    );
    host_answers(dir.path(), "amp2", "amp2-sheep");

    let run = run_guarded(dir.path(), &["amp1", "amp2"], ROOMY, VersionGuard::Enforce).await;

    assert_eq!(run.code, ExitCode::VersionSkew);
    assert!(!run.out.contains("amp1-sheep"), "{}", run.out);
    assert!(run.out.contains("amp2-sheep"), "{}", run.out);
    assert!(run.err.contains("amp1: this shep is "), "{}", run.err);
    assert!(
        run.err
            .contains(&format!("speaking protocol {}", PROTOCOL_VERSION - 1)),
        "{}",
        run.err
    );
    assert!(
        run.err.contains("ssh amp1 shep daemon reload"),
        "{}",
        run.err
    );
}

/// The fleet shape a patch release used to go dark over: a host on the
/// previous version, the wire unchanged. Under a guard that enforces it is
/// listed, with the same notice a read verb gets.
#[tokio::test]
async fn a_host_on_another_version_but_the_same_protocol_is_listed_when_the_guard_enforces() {
    let (dir, _held) = fixture().await;
    host_answers_as(dir.path(), "amp1", "0.0.1", "amp1-sheep");

    let run = run_guarded(dir.path(), &["amp1"], ROOMY, VersionGuard::Enforce).await;

    assert_eq!(run.code, ExitCode::Success, "{}", run.err);
    assert!(run.out.contains("amp1-sheep"), "{}", run.out);
    assert!(
        run.err.contains("the shepherd on amp1 is 0.0.1"),
        "{}",
        run.err
    );
    assert!(run.err.contains("The wire protocol matches"), "{}", run.err);
    assert!(
        !run.err.contains("error[version_skew]"),
        "a tolerated skew is a notice, not a refusal: {}",
        run.err
    );
}

#[tokio::test]
async fn a_host_on_another_version_is_listed_without_a_warning_when_the_guard_is_exempt() {
    let (dir, _held) = fixture().await;
    host_answers_as(dir.path(), "amp1", "0.0.1", "amp1-sheep");

    let run = run_guarded(dir.path(), &["amp1"], ROOMY, VersionGuard::Exempt).await;

    assert_eq!(run.code, ExitCode::Success, "{}", run.err);
    assert!(run.out.contains("amp1-sheep"), "{}", run.out);
    assert_eq!(run.err, "");
}

/// A stdout that has been closed under the run.
struct Closed;

impl std::io::Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::PermissionDenied.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_stdout_that_fails_while_the_roll_prints_ends_the_run_with_failure() {
    let dir = tempfile::tempdir().unwrap();
    write_ssh(dir.path());
    let paths = paths_in(dir.path());
    let roll =
        shep_daemon::snapshot::FlockSnapshot::with_apps(vec![shep_daemon::snapshot::SavedApp {
            app: shep_core::config::AppConfig::minimal("rolled-sheep", "./srv"),
            instances_running: 1,
            disabled: false,
        }]);
    std::fs::write(&paths.snapshot, serde_json::to_vec(&roll).unwrap()).unwrap();
    host_answers(dir.path(), "amp1", "amp1-sheep");
    let ssh = unattended(ROOMY).program(dir.path().join("ssh"));
    let mut err = Vec::new();

    let code = fleet(
        &mut Streams::bare(&mut Closed, &mut err, Format::Table),
        &paths,
        VersionGuard::Warn,
        &["amp1".to_owned()],
        &ssh,
        ROOMY,
    )
    .await;

    assert_eq!(code, ExitCode::Failure);
}

/// Dispatch tests: `shep flock` as `flock_command_on` reads this machine's
/// `shep.toml`.
mod dispatch {
    use super::*;

    async fn flock(paths: &ShepPaths, fmt: Format, follow: bool, terminal: bool) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = FlockArgs {
            follow,
            interval: crate::cli::FOLLOW_INTERVAL_FLOOR_SECONDS,
        };
        let code = flock_command_on(
            &mut Streams::bare(&mut out, &mut err, fmt),
            &Link::Local(paths),
            VersionGuard::Warn,
            &args,
            terminal,
        )
        .await;
        Run {
            code,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
        }
    }

    fn list_hosts(paths: &ShepPaths, toml: &str) {
        std::fs::write(&paths.daemon_config, toml).unwrap();
    }

    #[tokio::test]
    async fn flock_follow_with_a_hosts_list_dials_no_host() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        list_hosts(&paths, "hosts = [\"amp1\"]\n");

        // Not a terminal: the follow refuses before it connects anywhere, and
        // a fleet listing would have said something about amp1 or the roll.
        let run = flock(&paths, Format::Table, true, false).await;

        assert_eq!(run.code, ExitCode::Usage);
        assert_eq!(run.out, "");
        assert!(
            run.err.contains("`--follow` needs a terminal"),
            "{}",
            run.err
        );
        assert!(!run.err.contains("amp1"), "{}", run.err);
    }

    #[tokio::test]
    async fn a_shep_toml_that_will_not_parse_still_lists_the_local_flock_and_says_the_list_was_not_read()
     {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let _held = local_shepherd(&paths, env!("CARGO_PKG_VERSION"), "local-sheep").await;
        list_hosts(&paths, "hosts = [\"amp1\"\n[daemon\ntoken = \"hunter2\"\n");

        let run = flock(&paths, Format::Table, false, false).await;

        assert_eq!(run.code, ExitCode::Success, "{}", run.err);
        assert!(run.out.contains("local-sheep"), "{}", run.out);
        assert!(!run.out.starts_with("local\n"), "{}", run.out);
        assert!(run.err.contains("hosts list"), "{}", run.err);
        assert!(run.err.contains("was not read"), "{}", run.err);
        assert!(
            !run.err.contains("hunter2"),
            "the document stays out of the aside: {}",
            run.err
        );
        assert_eq!(run.err.lines().count(), 1, "{}", run.err);
    }

    #[tokio::test]
    async fn with_no_hosts_list_the_listing_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let _held = local_shepherd(&paths, env!("CARGO_PKG_VERSION"), "local-sheep").await;
        let client = Client::connect(&paths.socket).await.unwrap();
        let (_, expected) = crate::commands::query::testing::listing(&client, Format::Table).await;

        for toml in [None, Some("hosts = []\n")] {
            if let Some(toml) = toml {
                list_hosts(&paths, toml);
            }
            let run = flock(&paths, Format::Table, false, false).await;

            assert_eq!(run.code, ExitCode::Success, "{}", run.err);
            assert_eq!(run.out, expected);
            assert_eq!(run.err, "");
        }
    }
}
