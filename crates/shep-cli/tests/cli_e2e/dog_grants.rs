//! A dog's grants, written into `shep.toml` by hand and enforced by the
//! shepherd the real binary boots: the request outside them is refused with
//! the message the spec gives, the one inside them is not, and a dog with no
//! grants keeps full access.

use shep_core::protocol::{Envelope, PROTOCOL_VERSION, Request, SelectorSpec};

use super::*;

/// One request as the `printf` that writes its frame to the dog's fd 4: four
/// length bytes, then the envelope.
#[cfg(unix)]
fn send(id: u64, body: Request) -> String {
    let json = serde_json::to_string(&Envelope {
        id,
        deadline_ms: None,
        body,
    })
    .unwrap();
    assert!(
        json.len() < 256,
        "the script writes one length byte: {json}"
    );
    format!(
        "printf '\\000\\000\\000\\{:03o}%s' '{json}' >&4",
        json.len()
    )
}

/// Writes a `/bin/sh` dog that handshakes on fd 4, sends a `stop` and then a
/// `list`, and keeps each reply as `$SHEP_HOME/reply-<n>.json`.
#[cfg(unix)]
fn write_dog(dir: &TempDir) -> PathBuf {
    let hello = format!(r#"{{"client_version":"test","protocol":{PROTOCOL_VERSION}}}"#);
    let hello = format!(
        "printf '\\000\\000\\000\\{:03o}%s' '{hello}' >&4",
        hello.len()
    );
    let nobody = SelectorSpec::Name("nosuchsheep".to_string());
    let stop = send(1, Request::Stop { selector: nobody });
    let list = send(2, Request::ListFlock);
    let body = format!(
        r#"#!/bin/sh
case "$1" in
  --*) exit 0 ;;
esac
read_frame() {{
  n=$(dd bs=1 count=4 <&4 2>/dev/null | od -An -tu1 | awk 'NR == 1 {{print $3 * 256 + $4}}')
  dd bs=1 count="$n" <&4 2>/dev/null
}}
{hello}
read_frame > /dev/null
{stop}
read_frame > "$SHEP_HOME/reply-1.tmp" && mv "$SHEP_HOME/reply-1.tmp" "$SHEP_HOME/reply-1.json"
{list}
read_frame > "$SHEP_HOME/reply-2.tmp" && mv "$SHEP_HOME/reply-2.tmp" "$SHEP_HOME/reply-2.json"
exec sleep 60
"#
    );
    write_script(dir, "grantdog.sh", &body)
}

/// Waits for the dog's `n`th reply and returns its JSON.
#[cfg(unix)]
fn reply(home: &Path, n: u8) -> String {
    let path = home.join(format!("reply-{n}.json"));
    let deadline = Instant::now() + CMD_TIMEOUT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Boots a shepherd whose `shep.toml` adopts the dog, with `grants` hand
/// written under it when given, and returns the dog's two replies.
#[cfg(unix)]
fn replies_with(grants: Option<&str>) -> (String, String) {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let dog = write_dog(&dir);
    let grants = grants.map_or_else(String::new, |names| {
        format!("\n[daemon.dog_grants.grantdog]\ngranted = {names}\n")
    });
    std::fs::write(
        home.join("shep.toml"),
        format!(
            "[daemon]\nenabled_dogs = [\"grantdog\"]\n\n[daemon.adopted_dogs]\ngrantdog = \"{}\"\n{grants}",
            dog.display()
        ),
    )
    .unwrap();
    let mut guard = DaemonGuard::default();

    let mustered = shep(home).arg("muster").output().unwrap();
    guard.adopt_home(home);
    assert_success(&mustered);

    (reply(home, 1), reply(home, 2))
}

/// Granted `flock.read` and nothing else, the dog's `list` goes through and
/// its `stop` is refused in the words the spec gives.
#[cfg(unix)]
#[test]
fn a_dog_is_refused_outside_the_grants_written_into_shep_toml() {
    let (stop, list) = replies_with(Some(r#"["flock.read"]"#));

    assert!(stop.contains("forbidden"), "{stop}");
    assert!(
        stop.contains(
            "dog `grantdog` was refused `stop`: it needs `flock.control`, \
             which was not granted to it"
        ),
        "{stop}"
    );
    assert!(!list.contains("forbidden"), "{list}");
}

/// The control: the same dog with no `dog_grants` entry is undeclared.
#[cfg(unix)]
#[test]
fn a_dog_with_no_grants_written_keeps_full_access() {
    let (stop, list) = replies_with(None);

    assert!(!stop.contains("forbidden"), "{stop}");
    assert!(!list.contains("forbidden"), "{list}");
}
