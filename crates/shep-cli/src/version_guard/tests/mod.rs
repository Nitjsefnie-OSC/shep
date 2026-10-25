//! Shared fixtures for the version-guard suite, which is split by what the
//! link does: `refusal` for the local socket, `host` for `--host`, `notice`
//! for a difference a guard tolerates, and `verbs` for which guard each verb
//! gets.
//!
//! The fixtures live here because all four concerns reach the same three
//! things: a [`Client`] past a real handshake with a peer of a chosen version
//! and protocol, a [`Streams`] over two buffers so a refusal's exact text can
//! be read back, and the argument structs the verb-mapping tests parse.

use std::io::IsTerminal;

use cli::{DaemonArgs, StartArgs};
use shep_core::paths::ShepPaths;
use shep_core::protocol::{Request, Response};
use shep_core::status::ProcStatus;

use super::*;

/// A [`Streams`] over two byte buffers, so a refusal's exact text can be
/// read back, in the table form these tests drive.
fn buffered_streams<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>) -> Streams<'a> {
    Streams::bare(out, err, Format::Table)
}

/// A real [`Client`], past a real handshake, whose peer announced
/// `version`. The [`shep_client::testing::FakeDaemon`] is returned so it
/// outlives the client.
async fn client_announcing(
    addr: &std::path::Path,
    version: &str,
) -> (Client, shep_client::testing::FakeDaemon) {
    let ack = shep_core::protocol::HelloAck {
        daemon_version: version.to_owned(),
        protocol: shep_core::protocol::PROTOCOL_VERSION,
        pid: 4242,
        min_supported: None,
    };
    shep_client::testing::fake_client_with_ack(addr, ack).await
}

/// Writes an executable `ssh` into `dir` that answers the handshake with a
/// shepherd of `version` speaking `protocol`, then reads until the client
/// hangs up.
#[cfg(unix)]
fn ssh_stand_in(dir: &std::path::Path, version: &str, protocol: u32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;

    let ack = shep_core::protocol::HelloAck {
        daemon_version: version.to_owned(),
        protocol,
        pid: 4242,
        min_supported: None,
    };
    let reply: shep_core::protocol::HelloReply = Ok(ack);
    let payload = shep_core::protocol::encode_frame(&reply).unwrap();
    let mut framed = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    framed.extend_from_slice(&payload);
    std::fs::write(dir.join("reply.bin"), framed).unwrap();
    let program = dir.join("ssh");
    std::fs::write(
        &program,
        "#!/bin/sh\ncat \"$(dirname \"$0\")/reply.bin\"\ncat >/dev/null\n",
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    program
}

/// A [`Client`] dialled through a stand-in `ssh` that announced `version`
/// speaking `protocol`. `dir` has to outlive it: the stand-in is the file the
/// child is still reading.
#[cfg(unix)]
async fn client_over_host(dir: &std::path::Path, version: &str, protocol: u32) -> Client {
    let options = shep_client::SshOptions::default().program(ssh_stand_in(dir, version, protocol));
    Client::connect_ssh_with("amp2", &options)
        .await
        .expect("the stand-in ssh answers the handshake")
}

/// A home whose socket a fake shepherd on version `0.1.8` serves, listing
/// one sheep with `status`, and the channel of what it was sent.
async fn skewed_shepherd(
    dir: &std::path::Path,
    status: ProcStatus,
) -> (
    ShepPaths,
    tokio::sync::mpsc::UnboundedReceiver<shep_core::protocol::Envelope>,
) {
    let paths = ShepPaths::resolve(&|_| None, dir);
    std::fs::create_dir_all(&paths.run).unwrap();
    let mut info = shep_client::testing::sample_info();
    info.status = status;
    let ack = shep_core::protocol::HelloAck {
        daemon_version: "0.1.8".to_owned(),
        protocol: shep_core::protocol::PROTOCOL_VERSION,
        pid: 4242,
        min_supported: None,
    };
    let requests =
        shep_client::testing::fake_daemon_answering_with_ack(&paths.socket, ack, move |request| {
            match request {
                Request::ListFlock => Response::Flock(vec![info.clone()]),
                _ => Response::Pong,
            }
        })
        .await;
    // Held by the caller: the fake stops serving once nobody reads it.
    (paths, requests)
}

/// An empty [`StartArgs`], for asking which guard `add` gets. The guard
/// reads the verb alone, so no field here reaches it.
fn add_args() -> StartArgs {
    StartArgs {
        targets: Vec::new(),
        name: None,
        fold: None,
        cwd: None,
        interpreter: None,
        flockfile: false,
        no_log_timestamps: false,
        reset: None,
    }
}

/// A [`cli::FlockArgs`] that does not follow: the bare `shep flock`.
fn flock_args() -> cli::FlockArgs {
    cli::FlockArgs {
        follow: false,
        interval: cli::FOLLOW_INTERVAL_FLOOR_SECONDS,
    }
}

/// A [`DaemonArgs`] carrying `cmd` and nothing else, for asking which
/// guard the `daemon` verb's two shapes get.
fn daemon_args(cmd: Option<cli::DaemonCmd>) -> DaemonArgs {
    DaemonArgs {
        cmd,
        no_restore: false,
        foreground: false,
        log_json: None,
        log_level: None,
        socket: None,
        max_cron_sleep: None,
    }
}

/// The verb `argv` names, parsed the way the binary parses it.
fn verb(argv: &[&str]) -> Commands {
    use clap::Parser as _;

    let argv = std::iter::once("shep").chain(argv.iter().copied());
    cli::Cli::try_parse_from(argv).unwrap().command
}

mod host;
mod notice;
mod refusal;
mod verbs;
