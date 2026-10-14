//! A real sheep's channel samples, a real metrics dog and a real scrape.
//!
//! #733's own tests push events into a `shep_client::testing` fake, and a fake
//! delivers whatever it is handed, in the order it is handed it, without
//! spawning anything. These four run the ordering a fake cannot: a dog
//! subscribed before the sheep it watches samples, a restart's lifecycle
//! events against the new instance's first sample, a feed against the shepherd
//! a real `execve` put in place, and a refusal a real sheep actually sent.

use super::*;

/// The name every case here starts its one sheep under.
#[cfg(unix)]
const SHEEP: &str = "web";

/// A labelled counter: the two own labels are in the order the wire's
/// `BTreeMap` sorts them to, and `type` is the counter the scrape must render.
#[cfg(unix)]
const LABELLED_COUNTER: &str = r#"{"kind":"metric","name":"queue_depth","value":7,"type":"counter","labels":{"model":"a","route":"/x"}}"#;

/// A sample in the dog's own namespace, which the grammar refuses.
#[cfg(unix)]
const RESERVED_NAME_SAMPLE: &str =
    r#"{"kind":"metric","name":"shep_mine","value":1,"type":"counter"}"#;

/// A sample the same sheep sends beside the refused one, so a case can tell a
/// feed that refused one message from a feed that stopped.
#[cfg(unix)]
const PLAIN_SAMPLE: &str = r#"{"kind":"metric","name":"queue_depth","value":3,"type":"counter"}"#;

/// The one `boot_rps` the reload case sends, one second into its sheep's
/// life and never again.
#[cfg(unix)]
const BOOT_SAMPLE: &str = r#"{"kind":"metric","name":"boot_rps","value":5,"type":"gauge"}"#;

/// The `tick_rps` line inside that sheep's loop, with the shell's own `$v`
/// filling the `printf` `%d`: 1 until `phase-two` exists, 2 after it.
#[cfg(unix)]
const TICK_LOOP_LINE: &str = "printf '{\"kind\":\"metric\",\"name\":\"tick_rps\",\"value\":%d,\"type\":\"gauge\"}\\n' \"$v\" >&3\n";

/// Brings up a shepherd with the metrics dog enabled, spawned and answering a
/// scrape, before any sheep exists.
///
/// The order is the case, not a convenience: the bus delivers from the moment
/// a subscriber joins, so a dog enabled after a sheep's one sample could never
/// have seen it. `shep muster` autostarts the shepherd over an empty roll,
/// `shep enable metrics` spawns the dog on the live one, and a scrape that
/// answers `200` is the dog's own proof that it has bound and is talking to
/// this shepherd.
///
/// Returns that first body, for a caller that needs to know what the dog was
/// serving before its flock had anything in it.
#[cfg(unix)]
fn a_metrics_dog_answering(
    home: &Path,
    addr: std::net::SocketAddr,
    guard: &mut DaemonGuard,
) -> String {
    let mustered = shep(home).arg("muster").output().unwrap();
    // Registered before anything that can panic below, per the rule at the top
    // of `cli_e2e/main.rs`.
    guard.adopt_home(home);
    assert_success(&mustered);

    let enabled = shep(home).arg("enable").arg("metrics").output().unwrap();
    assert_success(&enabled);
    guard.adopt_dog_pid(wait_for_dog_pid(home, "metrics"));

    let body = poll_metrics_containing(addr, "HTTP/1.1 200");
    assert!(
        body.contains("HTTP/1.1 200"),
        "the dog must be answering BEFORE any sheep exists, or a sample it \
         never subscribed for would pass as one it kept: {body}"
    );
    body
}

/// A sheep script that runs `lines` and then stays up for
/// [`SCRIPT_SLEEP_SECS`], recording its own pid for [`DaemonGuard`] first.
#[cfg(unix)]
fn write_channel_script(dir: &TempDir, name: &str, lines: &str) -> PathBuf {
    write_script(
        dir,
        &format!("{name}.sh"),
        &format!(
            "{}{}{lines}{}",
            script_header(),
            record_pid_line(dir),
            sleep_line(SCRIPT_SLEEP_SECS)
        ),
    )
}

/// The line that writes one channel `message`: `printf` with the JSON as its
/// format string, on the fd `fixtures::ready_message_line` uses.
#[cfg(unix)]
fn channel_write_line(message: &str) -> String {
    format!("printf '{message}\\n' >&3\n")
}

/// A Flockfile whose one app asks for a shepherd channel, which is what puts
/// fd 3 under the script: `channel` is the one field that opens one without
/// any other behaviour implied.
#[cfg(unix)]
fn write_channel_flockfile(dir: &TempDir, script: &Path) -> PathBuf {
    write_flockfile(
        dir,
        &format!(
            "[[app]]\nname = \"{SHEEP}\"\nscript = '{}'\nchannel = true\n",
            script.display()
        ),
    )
}

/// The whole label list one of `SHEEP`'s samples renders with, braces
/// included: the labels this dog already uses for the process, then the
/// sample's own, in the order the metrics dog puts them.
///
/// `id` comes from the flock listing rather than a guess, which is the point:
/// a channel sample has to line up with the built-in series for the same
/// instance.
#[cfg(unix)]
fn sheep_series(name: &str, id: u64, own_labels: &[(&str, &str)]) -> String {
    let mut labels = format!(r#"sheep="{SHEEP}",id="{id}",fold="""#);
    for (key, value) in own_labels {
        labels.push_str(&format!(",{key}=\"{value}\""));
    }
    format!("{name}{{{labels}}}")
}

/// The value `shep_channel_samples_refused_total{reason="<reason>"}` answers
/// with, or `0` when the exposition carries no such series at all.
#[cfg(unix)]
fn refused_total(body: &str, reason: &str) -> u64 {
    let needle = format!(r#"shep_channel_samples_refused_total{{reason="{reason}"}} "#);
    body.lines()
        .filter_map(|line| line.strip_prefix(&needle))
        .find_map(|rest| rest.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

#[cfg(unix)]
/// Catches #733 storing a sample it never renders, rendering it without the
/// labels that tie it to the instance that sent it, or keeping it after that
/// instance was stopped.
#[test]
fn a_real_sheeps_samples_reach_a_real_scrape_and_go_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let port = free_port();
    write_shep_toml(
        &dir,
        &format!("[dog.metrics]\nbind = \"127.0.0.1:{port}\"\n"),
    );
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut guard = DaemonGuard::default();

    let before = a_metrics_dog_answering(home, addr, &mut guard);
    assert!(
        !before.contains("queue_depth"),
        "no sheep has been started yet, so the dog has no sample of one to \
         be serving: {before}"
    );

    let script = write_channel_script(&dir, "sample", &channel_write_line(LABELLED_COUNTER));
    let flockfile = write_channel_flockfile(&dir, &script);
    let started = shep(home).arg("start").arg(&flockfile).output().unwrap();
    assert_success(&started);

    let sheep = poll_flock(home, |info| info["status"] == "online");
    assert_eq!(sheep["name"], SHEEP, "{sheep}");
    let id = sheep["id"].as_u64().expect("an online sheep has an id");

    let series = sheep_series("queue_depth", id, &[("model", "a"), ("route", "/x")]);
    let body = poll_metrics_containing(addr, &series);
    assert!(
        body.contains(&format!("{series} 7\n")),
        "the scrape must carry the sample this sheep sent on its channel, \
         under this dog's own labels for the sender and the sample's own two, \
         at the value it carried: {body}"
    );
    assert!(
        body.contains("# TYPE queue_depth counter"),
        "the sample said `counter` on the wire, so the family must be \
         rendered as one: {body}"
    );

    let stopped = shep(home).arg("stop").arg(SHEEP).output().unwrap();
    assert_success(&stopped);
    let after = poll_metrics_without(addr, "queue_depth");
    // The absence proves nothing until this reading is the dog serving a real
    // exposition: a poll that expired hands back whatever its last attempt was,
    // a `503` or an empty string, and `shep_daemon_up` is the one series a `200`
    // carries whatever the flock holds.
    assert!(
        after.contains("HTTP/1.1 200") && after.contains("shep_daemon_up"),
        "the dog stopped serving an exposition, which is not the same event as \
         the sample leaving: {after}"
    );
    assert!(
        !after.contains("queue_depth"),
        "a stopped sheep is still reporting a value it stopped updating: {after}"
    );

    graceful_kill(home);
}

#[cfg(unix)]
/// Catches #733 letting a restarted sheep serve the sample the instance the
/// restart took out sent.
#[test]
fn a_restarted_sheep_does_not_serve_its_previous_instances_sample() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let port = free_port();
    write_shep_toml(
        &dir,
        &format!("[dog.metrics]\nbind = \"127.0.0.1:{port}\"\n"),
    );
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut guard = DaemonGuard::default();

    a_metrics_dog_answering(home, addr, &mut guard);

    // The marker picks which name this run sends, so the restarted instance
    // emits a series the first one never could have emitted at any time.
    let marker = dir.path().join("metric-name");
    std::fs::write(&marker, "first_run_rps").unwrap();
    let named =
        r#"printf '{"kind":"metric","name":"%s","value":1,"type":"counter"}\n' "$name" >&3"#;
    let script = write_channel_script(
        &dir,
        "marker",
        &format!("name=$(cat \"{}\")\n{named}\n", marker.display(),),
    );
    let flockfile = write_channel_flockfile(&dir, &script);

    let started = shep(home).arg("start").arg(&flockfile).output().unwrap();
    assert_success(&started);
    let first = poll_flock(home, |info| info["status"] == "online");
    let first_pid = first["pid"].as_u64().expect("an online sheep has a pid");

    let serving = poll_metrics_containing(addr, "first_run_rps");
    assert!(
        serving.contains("first_run_rps"),
        "the first instance's own sample never reached the scrape: {serving}"
    );

    std::fs::write(&marker, "second_run_rps").unwrap();
    let restarted = shep(home).arg("restart").arg(SHEEP).output().unwrap();
    assert_success(&restarted);
    let again = poll_flock(home, |info| {
        info["status"] == "online" && info["pid"].as_u64().is_some_and(|pid| pid != first_pid)
    });
    assert_ne!(
        again["pid"].as_u64(),
        Some(first_pid),
        "the sheep was never replaced, so this case restarted nothing: {again}"
    );

    let body = poll_metrics_containing(addr, "second_run_rps");
    assert!(
        body.contains("second_run_rps"),
        "the new instance's own sample never reached the scrape: {body}"
    );
    assert!(
        !body.contains("first_run_rps"),
        "the instance this restart put in place is serving the sample the \
         instance it took out sent: {body}"
    );

    graceful_kill(home);
}

#[cfg(unix)]
/// Catches #733 losing a flock's samples when the shepherd replaces itself,
/// or losing its feed on the successor so nothing refills them.
#[test]
fn a_sample_survives_a_real_daemon_reload() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let port = free_port();
    write_shep_toml(
        &dir,
        &format!("[dog.metrics]\nbind = \"127.0.0.1:{port}\"\n"),
    );
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut guard = DaemonGuard::default();

    a_metrics_dog_answering(home, addr, &mut guard);

    // One `boot_rps`, sent once, one second in, and never again, and a
    // `tick_rps` whose value is 1 until `phase-two` exists. Nothing this case
    // does after the reload makes the sheep send `boot_rps` a second time.
    let phase = dir.path().join("phase-two");
    let mut ticker = String::new();
    ticker.push_str("sleep 1\n");
    ticker.push_str(&channel_write_line(BOOT_SAMPLE));
    ticker.push_str("i=0\nwhile [ $i -lt 200 ]; do\n");
    ticker.push_str(&format!(
        "if [ -e \"{}\" ]; then v=2; else v=1; fi\n",
        phase.display()
    ));
    ticker.push_str(TICK_LOOP_LINE);
    ticker.push_str("i=$((i + 1))\nsleep 0.2\ndone\n");
    let script = write_channel_script(&dir, "ticker", &ticker);
    let flockfile = write_channel_flockfile(&dir, &script);

    let started = shep(home).arg("start").arg(&flockfile).output().unwrap();
    assert_success(&started);
    let sheep = poll_flock(home, |info| info["status"] == "online");
    let id = sheep["id"].as_u64().expect("an online sheep has an id");
    let pid = sheep["pid"].as_u64().expect("an online sheep has a pid");
    let tick = sheep_series("tick_rps", id, &[]);
    let boot = sheep_series("boot_rps", id, &[]);

    let before = poll_metrics_containing(addr, &tick);
    assert!(
        before.contains(&format!("{tick} 1\n")),
        "the series must be in the scrape BEFORE the reload, or the reload \
         proves nothing: {before}"
    );
    let boot_before = poll_metrics_containing(addr, &format!("{boot} 5\n"));
    assert!(
        boot_before.contains(&format!("{boot} 5\n")),
        "the one `boot_rps` this sheep sends, at the value it sent it at, must \
         already be in the scrape, or there is nothing for the reload to have \
         kept: {boot_before}"
    );

    let reloaded = shep(home).arg("daemon").arg("reload").output().unwrap();
    assert_success(&reloaded);

    // The decisive ordering: the sheep this case is watching is still the one
    // that sent `boot_rps`, so only the dog keeping what it had can answer for
    // it. A stop-and-start would put a fresh instance on the slot, and a fresh
    // instance would simply send everything again.
    let carried = poll_flock(home, |info| {
        info["status"] == "online" && info["pid"].as_u64().is_some_and(|p| p == pid)
    });
    assert_eq!(
        carried["pid"].as_u64(),
        Some(pid),
        "the reload restarted this sheep rather than carrying it, so what is \
         being served now could have been re-sent: {carried}"
    );

    let after = poll_metrics_containing(addr, &format!("{boot} 5\n"));
    assert!(
        after.contains(&format!("{boot} 5\n")),
        "the reload left no `boot_rps` for the instance it put in place, or left \
         one at a value the sheep never sent: {after}"
    );

    // The value moves only now, and only a feed armed on the successor can
    // carry it: a dog serving what it cached before the exec keeps answering 1.
    std::fs::write(&phase, "").unwrap();
    let moved = poll_metrics_containing(addr, &format!("{tick} 2\n"));
    assert!(
        moved.contains(&format!("{tick} 2\n")),
        "no sample this sheep sent after the reload reached the scrape, which \
         is a dog whose subscription died with the shepherd that \
         `execve`'d: {moved}"
    );

    graceful_kill(home);
}

#[cfg(unix)]
/// Catches #733 serving a sample it says it refuses, and refusing one without
/// counting it anywhere.
#[test]
fn a_sample_naming_shep_underscore_is_refused_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let port = free_port();
    write_shep_toml(
        &dir,
        &format!("[dog.metrics]\nbind = \"127.0.0.1:{port}\"\n"),
    );
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut guard = DaemonGuard::default();

    a_metrics_dog_answering(home, addr, &mut guard);

    let script = write_channel_script(
        &dir,
        "reserved",
        &(channel_write_line(RESERVED_NAME_SAMPLE) + &channel_write_line(PLAIN_SAMPLE)),
    );
    let flockfile = write_channel_flockfile(&dir, &script);

    let started = shep(home).arg("start").arg(&flockfile).output().unwrap();
    assert_success(&started);
    let sheep = poll_flock(home, |info| info["status"] == "online");
    let id = sheep["id"].as_u64().expect("an online sheep has an id");

    // One reading carrying both needles. The refused message is the first of
    // the two deliveries, so a scrape that lands between them answers the
    // counter and not yet the sample sent beside it.
    let refused = r#"shep_channel_samples_refused_total{reason="reserved_name"}"#;
    let kept = format!("{} 3\n", sheep_series("queue_depth", id, &[]));
    let body = poll_metrics_containing_all(addr, &[refused, &kept]);
    assert!(
        !body.contains("shep_mine"),
        "the dog serves its own `shep_` namespace and refused this sample by \
         name, so nothing named `shep_mine` may come back out of it: {body}"
    );
    assert!(
        body.contains(&kept),
        "one refused sample stopped the feed rather than that one \
         sample: {body}"
    );
    assert!(
        refused_total(&body, "reserved_name") > 0,
        "a metric that simply never appears is silent without this counter: {body}"
    );

    graceful_kill(home);
}
