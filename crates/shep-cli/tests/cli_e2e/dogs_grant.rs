//! `shep dogs revoke` against a shepherd the real binary booted: the dog's
//! next request needing the revoked permission is refused with no restart,
//! and a grant the dog never declared is refused before it is written.

use shep_core::protocol::{PROTOCOL_VERSION, Request, SelectorSpec};

use super::dog_grants::{reply, send};
use super::*;

/// Writes a `/bin/sh` dog that handshakes on fd 4, sends a `stop`, waits for
/// `$SHEP_HOME/go`, then sends another `stop`, keeping each reply as
/// `$SHEP_HOME/reply-<n>.json`. Its manifest declares `flock.control`.
#[cfg(unix)]
fn write_dog(dir: &TempDir) -> PathBuf {
    let hello = format!(r#"{{"client_version":"test","protocol":{PROTOCOL_VERSION}}}"#);
    let hello = format!(
        "printf '\\000\\000\\000\\{:03o}%s' '{hello}' >&4",
        hello.len()
    );
    let nobody = SelectorSpec::Name("nosuchsheep".to_string());
    let first = send(
        1,
        Request::Stop {
            selector: nobody.clone(),
        },
    );
    let second = send(2, Request::Stop { selector: nobody });
    let manifest = format!(
        r#"{{"manifest":1,"name":"grantdog","version":"1","protocol":{PROTOCOL_VERSION},"permissions":[{{"name":"flock.control","required":true,"reason":"stops sheep"}}]}}"#
    );
    let body = format!(
        r#"#!/bin/sh
case "$1" in
  --shep-manifest) echo '{manifest}'; exit 0 ;;
  --*) exit 0 ;;
esac
read_frame() {{
  n=$(dd bs=1 count=4 <&4 2>/dev/null | od -An -tu1 | awk 'NR == 1 {{print $3 * 256 + $4}}')
  dd bs=1 count="$n" <&4 2>/dev/null
}}
{hello}
read_frame > /dev/null
{first}
read_frame > "$SHEP_HOME/reply-1.tmp" && mv "$SHEP_HOME/reply-1.tmp" "$SHEP_HOME/reply-1.json"
while [ ! -e "$SHEP_HOME/go" ]; do sleep 0.05; done
{second}
read_frame > "$SHEP_HOME/reply-2.tmp" && mv "$SHEP_HOME/reply-2.tmp" "$SHEP_HOME/reply-2.json"
exec sleep 60
"#
    );
    write_script(dir, "grantdog.sh", &body)
}

/// A home whose `shep.toml` adopts and enables the dog with `granted`.
#[cfg(unix)]
fn home_granting(dir: &TempDir, granted: &str) -> PathBuf {
    let home = dir.path().to_path_buf();
    let dog = write_dog(dir);
    std::fs::write(
        home.join("shep.toml"),
        format!(
            "[daemon]\nenabled_dogs = [\"grantdog\"]\n\n[daemon.adopted_dogs]\ngrantdog = \"{}\"\n\n\
             [daemon.dog_grants.grantdog]\ngranted = {granted}\n",
            dog.display()
        ),
    )
    .unwrap();
    home
}

/// Granted `flock.control`, the dog's first `stop` goes through. Revoked
/// while the shepherd keeps running, its second `stop` is refused.
#[cfg(unix)]
#[test]
fn a_revoke_refuses_the_running_dogs_next_request_with_no_restart() {
    let dir = tempfile::tempdir().unwrap();
    let home = home_granting(&dir, r#"["flock.control"]"#);
    let mut guard = DaemonGuard::default();
    let mustered = shep(&home).arg("muster").output().unwrap();
    guard.adopt_home(&home);
    assert_success(&mustered);
    let before = reply(&home, 1);

    let revoked = shep(&home)
        .args(["dogs", "revoke", "grantdog", "flock.control"])
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();
    assert_success(&revoked);
    std::fs::write(home.join("go"), "").unwrap();
    let after = reply(&home, 2);

    assert!(!before.contains("forbidden"), "{before}");
    assert!(
        after.contains(
            "dog `grantdog` was refused `stop`: it needs `flock.control`, \
             which was not granted to it"
        ),
        "{after}"
    );
}

/// The permission is real but the dog's manifest does not list it.
#[cfg(unix)]
#[test]
fn a_grant_the_dog_never_declared_is_refused_and_leaves_shep_toml_alone() {
    let dir = tempfile::tempdir().unwrap();
    let home = home_granting(&dir, r#"["flock.control"]"#);
    let before = std::fs::read_to_string(home.join("shep.toml")).unwrap();

    let output = shep(&home)
        .args(["dogs", "grant", "grantdog", "flock.read"])
        .timeout(CMD_TIMEOUT)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        said.contains("grantdog did not declare `flock.read`"),
        "{said}"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("shep.toml")).unwrap(),
        before
    );
}
