//! A dog that answers `shep.validate`, end to end: the manifest that asks
//! for it, the channel it is asked over, and the write its refusal stops.
//!
//! One flock, because the cost here is a daemon, a sheep and two live dogs.
//! `kelpie` declares `validate` and refuses any table holding the value
//! `banned`; `quiet` declares nothing and is the control that a write nobody
//! was asked about still lands.
//!
//! Every accepted write is proof of its own round trip: a dog that answered
//! nothing is refused by the timeout, so a write that landed was one the dog
//! said yes to.

use super::*;

/// The dog's own sentence, which is what the operator has to read.
const REASON: &str = "checkout may not point outside the project";

/// The value this dog objects to. It is in the Flockfile from the start, so
/// the refusals need no write to set them up, and no accepted write can ever
/// put one in.
#[cfg(unix)]
const BANNED: &str = "banned";

/// `/bin/sh` for a dog that asks for the channel, answers
/// [`MANIFEST_FLAG`](shep_core::dogs::MANIFEST_FLAG) with `__MANIFEST__`, and
/// answers every action it is sent. A `shep.validate` whose params hold
/// `banned` is refused in the dog's own words; every other one is accepted.
///
/// Each reply is `printf '%s\n'`, not a bare format string: a reply body is
/// JSON inside a JSON string, and `printf` would eat the backslashes that
/// quote it.
#[cfg(unix)]
const DOG_BODY: &str = r#"#!/bin/sh
case "$1" in
  --version) printf 'shep-fixture 0.1.0\nshep-channel: true\n'; exit 0 ;;
  --shep-manifest) printf '%s\n' '__MANIFEST__'; exit 0 ;;
  --*) exit 0 ;;
esac
if [ -n "$SHEP_CHANNEL_FD" ]; then
  while IFS= read -r line <&3; do
    case "$line" in
      *'"shutdown"'*) exit 0 ;;
      *'"name":"shep.validate"'*)
        case "$line" in
          *__BANNED__*)
            printf '%s\n' '{"kind":"action-reply","action":"shep.validate","body":"{\"accept\":false,\"reason\":\"__REASON__\"}"}' >&3
          ;;
          *)
            printf '%s\n' '{"kind":"action-reply","action":"shep.validate","body":"{\"accept\":true}"}' >&3
          ;;
        esac
      ;;
      *) printf '{"kind":"action-reply","action":"gc","body":"swept"}\n' >&3 ;;
    esac
  done
fi
while :; do sleep 1 & wait $!; done
"#;

/// A dog binary whose manifest declares `validate` over its channel when
/// `validates`, and declares neither when it does not.
#[cfg(unix)]
fn write_validating_dog(dir: &TempDir, name: &str, validates: bool) -> PathBuf {
    let manifest = serde_json::json!({
        "manifest": 1,
        "name": name,
        "version": "0.1.0",
        "protocol": shep_core::protocol::PROTOCOL_VERSION,
        "channel": true,
        "validate": validates,
    })
    .to_string();
    let body = DOG_BODY
        .replace("__MANIFEST__", &manifest)
        .replace("__REASON__", REASON)
        .replace("__BANNED__", BANNED);
    write_script(dir, &format!("{name}.sh"), &body)
}

/// The whole arc over one flock: a refused write leaves the table as it was,
/// an accepted one lands, and a dog that declared nothing is never asked.
#[cfg(unix)]
#[test]
fn a_dog_that_refuses_its_settings_stops_the_write_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let script = write_test_script(&dir);
    let flockfile = write_flockfile(
        &dir,
        &format!(
            "[[app]]\nname = \"api\"\nscript = '{}'\n\n\
             [app.dogs.kelpie.git]\ncheckout = \"/srv/api\"\nnote = \"{BANNED}\"\n\n\
             [app.dogs.quiet.git]\nnote = \"{BANNED}\"\n\n\
             [[app]]\nname = \"web\"\nscript = '{}'\n\n\
             [app.dogs.kelpie.git]\nnote = \"kept\"\n",
            script.display(),
            script.display()
        ),
    );
    let validating = write_validating_dog(&dir, "kelpie", true);
    let plain = write_validating_dog(&dir, "quiet", false);

    let mut guard = DaemonGuard::default();
    let boot = shep(home).arg("start").arg(&flockfile).output().unwrap();
    guard.adopt_home(home);
    assert_success(&boot);
    poll_flock(home, |info| info["status"] == "online");

    for (dog, name) in [(&validating, "kelpie"), (&plain, "quiet")] {
        let adopted = shep(home)
            .arg("adopt")
            .arg(dog)
            .arg("--name")
            .arg(name)
            .output()
            .unwrap();
        assert_success(&adopted);
        wait_for_dog_pid(home, name);
    }

    let as_first_written = "[dogs.kelpie.git]\ncheckout = \"/srv/api\"\nnote = \"banned\"\n";

    // A `shep set` the dog refuses: the dog's own sentence on stderr, an exit
    // that says the settings were refused, and the table left as it was.
    let refused = shep(home)
        .arg("set")
        .arg("api.dogs.kelpie.git.checkout")
        .arg("/srv/other")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(4), "InvalidConfig; {refused:?}");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("nothing was written"), "{stderr}");
    assert!(stderr.contains(REASON), "the dog's own words: {stderr}");
    assert!(stderr.contains("`kelpie`"), "{stderr}");
    assert_eq!(read_table(home, "api.dogs.kelpie.git"), as_first_written);

    // `shep unset` is asked about the table it would leave behind, which here
    // still holds the value the dog objects to.
    let refused_removal = shep(home)
        .arg("unset")
        .arg("api.dogs.kelpie.git.checkout")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(
        refused_removal.status.code(),
        Some(4),
        "InvalidConfig; {refused_removal:?}"
    );
    assert!(
        String::from_utf8_lossy(&refused_removal.stderr).contains(REASON),
        "{refused_removal:?}"
    );
    assert_eq!(
        read_table(home, "api.dogs.kelpie.git"),
        as_first_written,
        "a refused removal removes nothing"
    );

    // `shep copy` carries a dog's whole table onto another sheep, which is a
    // write to that dog's settings through a door an operator would not think
    // of as one. The dog is asked about the table the target would hold.
    let copied_refused = shep(home)
        .arg("copy")
        .arg("api.dogs.kelpie.git")
        .arg("web.dogs.kelpie.git")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(
        copied_refused.status.code(),
        Some(4),
        "InvalidConfig; {copied_refused:?}"
    );
    let stderr = String::from_utf8_lossy(&copied_refused.stderr);
    assert!(
        stderr.contains("nothing was written") && stderr.contains(REASON),
        "the dog's own words, on a copy: {stderr}"
    );
    assert_eq!(
        read_table(home, "web.dogs.kelpie.git"),
        "[dogs.kelpie.git]\nnote = \"kept\"\n",
        "a copy the dog refused wrote nothing to the target"
    );

    // Removing the key the dog objects to is accepted, and it lands.
    let cleared = shep(home)
        .arg("unset")
        .arg("api.dogs.kelpie.git.note")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&cleared);
    assert_eq!(
        read_table(home, "api.dogs.kelpie.git"),
        "[dogs.kelpie.git]\ncheckout = \"/srv/api\"\n"
    );

    // The same write, now that the table is one the dog accepts.
    let accepted = shep(home)
        .arg("set")
        .arg("api.dogs.kelpie.git.checkout")
        .arg("/srv/other")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&accepted);
    assert_eq!(
        read_table(home, "api.dogs.kelpie.git"),
        "[dogs.kelpie.git]\ncheckout = \"/srv/other\"\n"
    );

    // The same copy, now that the table is one the dog accepts: it lands on the
    // target, key by key, keeping the key only the target had.
    let copied = shep(home)
        .arg("copy")
        .arg("api.dogs.kelpie.git")
        .arg("web.dogs.kelpie.git")
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&copied);
    let web_now = read_table(home, "web.dogs.kelpie.git");
    assert!(
        web_now.contains("checkout = \"/srv/other\"") && web_now.contains("note = \"kept\""),
        "{web_now}"
    );
    assert_eq!(
        read_table(home, "api.dogs.kelpie.git"),
        "[dogs.kelpie.git]\ncheckout = \"/srv/other\"\n",
        "a copy reads the source and never edits it"
    );

    // The control: the same value the dog objects to, a dog that declared
    // nothing. Nothing was asked, so nothing refused it.
    let unasked = shep(home)
        .arg("set")
        .arg("api.dogs.quiet.git.note")
        .arg(BANNED)
        .arg("--yes")
        .output()
        .unwrap();
    assert_success(&unasked);
    assert_eq!(
        read_table(home, "api.dogs.quiet.git"),
        "[dogs.quiet.git]\nnote = \"banned\"\n"
    );

    // A manifest shep cannot read answers no verdict. The adopted binary is
    // what the manifest is read from, so a dog whose binary is gone is a dog
    // whose `validate` cannot be checked, and that stops the write rather than
    // passing for a dog that declared nothing.
    std::fs::remove_file(&validating).expect("the adopted dog's binary");

    let unreadable = shep(home)
        .arg("set")
        .arg("api.dogs.kelpie.git.checkout")
        .arg("/srv/nowhere")
        .arg("--yes")
        .output()
        .unwrap();
    assert_eq!(
        unreadable.status.code(),
        Some(4),
        "InvalidConfig; {unreadable:?}"
    );
    let stderr = String::from_utf8_lossy(&unreadable.stderr);
    assert!(
        stderr.contains("nothing was written")
            && stderr.contains("cannot tell whether `kelpie` wants to be consulted"),
        "{stderr}"
    );
    assert_eq!(
        read_table(home, "api.dogs.kelpie.git"),
        "[dogs.kelpie.git]\ncheckout = \"/srv/other\"\n",
        "a manifest shep cannot read writes nothing"
    );

    for name in ["kelpie", "quiet"] {
        assert_success(&shep(home).arg("disable").arg(name).output().unwrap());
    }
    graceful_kill(home);
}

/// `shep get` for one address, as the exact text it prints.
#[cfg(unix)]
fn read_table(home: &Path, address: &str) -> String {
    let read = shep(home).arg("get").arg(address).output().unwrap();
    assert_success(&read);
    String::from_utf8_lossy(&read.stdout).into_owned()
}
