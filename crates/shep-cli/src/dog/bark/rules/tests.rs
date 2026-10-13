use shep_core::protocol::ProcessInfo;

use super::*;

fn one_sink(name: &str) -> BTreeMap<String, Sink> {
    let mut sinks = BTreeMap::new();
    sinks.insert(
        name.to_owned(),
        Sink::Json {
            url: "http://localhost/hook".to_owned(),
            body: None,
        },
    );
    sinks
}

/// The seam that matters: an operator hears about this when
/// `dogs.toml` is read, not when a rule first fires days later.
#[test]
fn a_sink_url_carrying_credentials_is_refused_when_the_rules_are_built() {
    let mut sinks = BTreeMap::new();
    sinks.insert(
        "ops".to_owned(),
        Sink::Json {
            url: "http://user:hunter2@localhost/hook".to_owned(),
            body: None,
        },
    );
    let err = Rules::new(Vec::new(), &sinks).unwrap_err();
    assert!(
        matches!(
            err,
            RulesError::UnusableSink(SinkConfigError::UrlCredentials { .. })
        ),
        "{err:?}"
    );
    assert!(!format!("{err} {err:?}").contains("hunter2"));
}

fn base_info(name: &str, status: ProcStatus) -> ProcessInfo {
    ProcessInfo::builder(1, name, status)
        .pid(Some(4242))
        .uptime_ms(1_000)
        .build()
}

fn errored_info(name: &str) -> ProcessInfo {
    base_info(name, ProcStatus::Errored)
}

fn online_info(name: &str) -> ProcessInfo {
    base_info(name, ProcStatus::Online)
}

fn process_event(name: &str, kind: ProcessEventKind) -> BusEvent {
    BusEvent::Process {
        event: kind,
        info: base_info(name, ProcStatus::Online),
        manually: false,
        at_ms: 0,
    }
}

fn errored_event(name: &str) -> BusEvent {
    process_event(name, ProcessEventKind::Errored)
}

fn restart_event(name: &str) -> BusEvent {
    process_event(name, ProcessEventKind::Restart)
}

fn gave_up_rules() -> Rules {
    let sinks = one_sink("ops");
    Rules::new(
        vec![Rule {
            when: Trigger::GaveUp {},
            sinks: vec!["ops".to_owned()],
            debounce: default_debounce(),
        }],
        &sinks,
    )
    .unwrap()
}

fn restart_rate_rules(restarts: u32, within: UpDuration) -> Rules {
    let sinks = one_sink("ops");
    Rules::new(
        vec![Rule {
            when: Trigger::RestartRate { restarts, within },
            sinks: vec!["ops".to_owned()],
            debounce: default_debounce(),
        }],
        &sinks,
    )
    .unwrap()
}

fn rule_to(sink: &str) -> Rule {
    Rule {
        when: Trigger::GaveUp {},
        sinks: vec![sink.to_owned()],
        debounce: default_debounce(),
    }
}

#[test]
fn an_errored_seen_by_both_routes_fires_once() {
    let mut rules = gave_up_rules();
    let first = rules.on_event(&errored_event("web"), 1_000);
    assert_eq!(first.len(), 1);
    let second = rules.on_poll(&[errored_info("web")], 2_000);
    assert!(second.is_empty(), "the debounce covers the other route");
}

#[test]
fn the_poll_fires_what_the_bus_never_carried() {
    let mut rules = gave_up_rules();
    let fired = rules.on_poll(&[errored_info("web")], 1_000);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].bark.subject, "web");
}

#[test]
fn one_flapping_sheep_does_not_mute_another_going_down() {
    let mut rules = gave_up_rules();
    assert_eq!(rules.on_event(&errored_event("web"), 1_000).len(), 1);
    assert_eq!(rules.on_event(&errored_event("api"), 1_100).len(), 1);
    assert!(rules.on_event(&errored_event("web"), 1_200).is_empty());
}

/// `info.restarts` says 9; only 3 restart events are fed through
/// `on_event`, so passing requires reading `info.restarts` rather than
/// a tally kept from the events.
#[test]
fn the_early_warning_counts_the_shepherds_restarts_and_not_its_own() {
    let mut rules = restart_rate_rules(5, UpDuration::from_millis(60_000));
    for at in [1_000, 2_000, 3_000] {
        let _ = rules.on_event(&restart_event("web"), at);
    }
    let mut info = online_info("web");
    info.restarts = 9;
    let fired = rules.on_poll(&[info], 4_000);
    assert_eq!(
        fired.len(),
        1,
        "9 restarts crosses a threshold of 5; 3 does not"
    );
}

#[test]
fn a_rule_routed_at_a_sink_that_does_not_exist_is_refused_at_startup() {
    let err = Rules::new(vec![rule_to("pager")], &BTreeMap::new()).unwrap_err();
    assert!(matches!(err, RulesError::UnknownSink { .. }));
    // Exact, not a `contains`: this string reaches an operator through
    // `Display`.
    assert_eq!(
        err.to_string(),
        "rule 0 routes to sink \"pager\", which [bark.sinks] in dogs.toml does not define"
    );
}

#[test]
fn a_bark_with_sinks_and_no_rules_still_alerts_when_the_shepherd_gives_up() {
    let sinks = one_sink("ops");
    let rules = Rules::default_rules(&sinks);
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].when, Trigger::GaveUp {});
    assert_eq!(rules[0].sinks, vec!["ops"]);
}

#[test]
fn a_rule_with_no_sinks_at_all_is_refused_at_startup() {
    let rule = Rule {
        when: Trigger::GaveUp {},
        sinks: Vec::new(),
        debounce: default_debounce(),
    };
    let err = Rules::new(vec![rule], &one_sink("ops")).unwrap_err();
    assert!(matches!(err, RulesError::NoSinks { .. }));
}

#[test]
fn an_event_rule_naming_an_unknown_kind_is_refused_at_startup() {
    let rule = Rule {
        when: Trigger::Event {
            kinds: vec!["exit".to_owned(), "not_a_real_kind".to_owned()],
        },
        sinks: vec!["ops".to_owned()],
        debounce: default_debounce(),
    };
    let err = Rules::new(vec![rule], &one_sink("ops")).unwrap_err();
    assert!(matches!(err, RulesError::UnknownKind { .. }));
    assert!(err.to_string().contains("not_a_real_kind"));
}

#[test]
fn an_event_rule_does_not_fire_on_a_kind_it_was_not_given() {
    let sinks = one_sink("ops");
    let mut rules = Rules::new(
        vec![Rule {
            when: Trigger::Event {
                kinds: vec!["exit".to_owned()],
            },
            sinks: vec!["ops".to_owned()],
            debounce: default_debounce(),
        }],
        &sinks,
    )
    .unwrap();
    let online = rules.on_event(&process_event("web", ProcessEventKind::Online), 1_000);
    assert!(online.is_empty(), "online was not in the configured kinds");
    let exit = rules.on_event(&process_event("web", ProcessEventKind::Exit), 1_100);
    assert_eq!(exit.len(), 1, "exit was, and should still fire");
}

/// Each rule routes to a sink of its own, so the firings name the rules
/// that fired.
#[test]
fn on_event_walks_a_mixed_rule_set_and_fires_only_the_matching_rules() {
    let rule = |when: Trigger, sink: &str| Rule {
        when,
        sinks: vec![sink.to_owned()],
        debounce: default_debounce(),
    };
    let mut sinks = BTreeMap::new();
    for name in ["event", "gave_up", "restart_rate", "memory"] {
        sinks.extend(one_sink(name));
    }
    let mut rules = Rules::new(
        vec![
            rule(
                Trigger::Event {
                    kinds: vec!["errored".to_owned()],
                },
                "event",
            ),
            rule(Trigger::GaveUp {}, "gave_up"),
            rule(
                Trigger::RestartRate {
                    restarts: 1,
                    within: UpDuration::from_millis(60_000),
                },
                "restart_rate",
            ),
            rule(
                Trigger::MemoryAbove {
                    bytes: MemSize::from_bytes(1),
                },
                "memory",
            ),
        ],
        &sinks,
    )
    .unwrap();

    let fired = rules.on_event(&errored_event("web"), 1_000);

    let routes: Vec<&[String]> = fired.iter().map(|f| f.sinks.as_slice()).collect();
    assert_eq!(
        routes,
        [["event".to_owned()], ["gave_up".to_owned()]],
        "the Event and GaveUp rules fire, in rule order"
    );
}

#[test]
fn restart_rate_fires_at_the_threshold_and_not_one_below_it() {
    let mut rules = restart_rate_rules(5, UpDuration::from_millis(60_000));
    let mut below = online_info("web");
    below.restarts = 4;
    assert!(
        rules.on_poll(&[below], 1_000).is_empty(),
        "4 restarts is below a threshold of 5"
    );
    let mut at = online_info("web");
    at.restarts = 5;
    assert_eq!(
        rules.on_poll(&[at], 1_100).len(),
        1,
        "5 restarts meets a threshold of 5"
    );
}

/// Debounce is zeroed: only the window's own logic keeps this quiet.
#[test]
fn restart_rate_window_slides_once_it_elapses() {
    let sinks = one_sink("ops");
    let mut rules = Rules::new(
        vec![Rule {
            when: Trigger::RestartRate {
                restarts: 5,
                within: UpDuration::from_millis(1_000),
            },
            sinks: vec!["ops".to_owned()],
            debounce: UpDuration::from_millis(0),
        }],
        &sinks,
    )
    .unwrap();

    let mut info = online_info("web");
    info.restarts = 5;
    assert_eq!(
        rules.on_poll(&[info.clone()], 0).len(),
        1,
        "5 restarts opens the window past threshold"
    );

    // Window elapsed and the count did not move: it resets, with
    // nothing new to warn about.
    assert!(
        rules.on_poll(&[info.clone()], 2_000).is_empty(),
        "no new restarts since the window reset"
    );

    // Five more restarts inside the new window crosses it again.
    info.restarts = 10;
    assert_eq!(
        rules.on_poll(&[info], 2_100).len(),
        1,
        "5 more restarts inside the new window crosses it again"
    );
}

#[test]
fn memory_above_fires_at_the_ceiling_and_not_one_byte_below_it() {
    let sinks = one_sink("ops");
    let mut rules = Rules::new(
        vec![Rule {
            when: Trigger::MemoryAbove {
                bytes: MemSize::from_bytes(1_000),
            },
            sinks: vec!["ops".to_owned()],
            debounce: default_debounce(),
        }],
        &sinks,
    )
    .unwrap();

    let mut below = online_info("web");
    below.memory_bytes = Some(999);
    assert!(
        rules.on_poll(&[below], 1_000).is_empty(),
        "999 is below 1000"
    );

    let mut at = online_info("web");
    at.memory_bytes = Some(1_000);
    assert_eq!(rules.on_poll(&[at], 1_100).len(), 1, "1000 meets 1000");
}

/// Unknown memory (a stopped sheep, or one not yet sampled) must read
/// as "cannot alert", never as zero.
#[test]
fn memory_above_does_not_fire_when_usage_is_unknown() {
    let sinks = one_sink("ops");
    let mut rules = Rules::new(
        vec![Rule {
            when: Trigger::MemoryAbove {
                bytes: MemSize::from_bytes(1_000),
            },
            sinks: vec!["ops".to_owned()],
            debounce: default_debounce(),
        }],
        &sinks,
    )
    .unwrap();
    let info = online_info("web");
    assert!(info.memory_bytes.is_none());
    assert!(rules.on_poll(&[info], 1_000).is_empty());
}

#[test]
fn gave_up_does_not_fire_on_event_for_a_non_errored_kind() {
    let mut rules = gave_up_rules();
    let online = rules.on_event(&process_event("web", ProcessEventKind::Online), 1_000);
    assert!(online.is_empty(), "GaveUp fires on Errored only");
    let restart = rules.on_event(&restart_event("web"), 1_100);
    assert!(restart.is_empty(), "GaveUp fires on Errored only");
}

#[test]
fn gave_up_does_not_fire_on_poll_for_a_non_errored_status() {
    let mut rules = gave_up_rules();
    let fired = rules.on_poll(&[online_info("web")], 1_000);
    assert!(fired.is_empty(), "GaveUp fires when status is Errored only");
}

#[test]
fn debounce_boundary_is_inclusive_at_exactly_its_own_duration() {
    let mut rules = gave_up_rules();
    let debounce_ms = default_debounce().as_millis();
    assert_eq!(rules.on_event(&errored_event("web"), 0).len(), 1);
    assert!(
        rules
            .on_event(&errored_event("web"), debounce_ms - 1)
            .is_empty(),
        "one millisecond short of the debounce must still be quiet"
    );
    assert_eq!(
        rules.on_event(&errored_event("web"), debounce_ms).len(),
        1,
        "exactly at the debounce it may fire again"
    );
}

// The tests above build `Rule`/`Trigger` as Rust values, never running
// `Deserialize`. The tests below parse real TOML strings.

/// The exact shape `docs/dogs.md` and `web/src/pages/docs/dogs.astro`
/// publish as copy-pasteable.
#[test]
fn the_docs_gave_up_rule_parses_from_toml() {
    let rule: Rule = toml::from_str(
        r#"
on = "gave_up"
sinks = ["oncall", "audit"]
"#,
    )
    .unwrap();
    assert_eq!(rule.when, Trigger::GaveUp {});
    assert_eq!(rule.sinks, vec!["oncall", "audit"]);
    assert_eq!(rule.debounce, default_debounce(), "no override in the TOML");
}

/// `within`'s `"2m"` form uses [`UpDuration`]'s own duration grammar.
#[test]
fn the_docs_restart_rate_rule_parses_from_toml() {
    let rule: Rule = toml::from_str(
        r#"
on = "restart_rate"
restarts = 5
within = "2m"
sinks = ["oncall"]
"#,
    )
    .unwrap();
    assert_eq!(
        rule.when,
        Trigger::RestartRate {
            restarts: 5,
            within: UpDuration::from_millis(2 * 60_000),
        }
    );
}

/// Not in the published docs, but a real `Trigger` variant a rule can
/// name.
#[test]
fn an_event_rule_parses_from_toml() {
    let rule: Rule = toml::from_str(
        r#"
on = "event"
kinds = ["exit", "errored"]
sinks = ["oncall"]
"#,
    )
    .unwrap();
    assert_eq!(
        rule.when,
        Trigger::Event {
            kinds: vec!["exit".to_owned(), "errored".to_owned()],
        }
    );
}

/// `bytes`'s `"512M"` form uses [`MemSize`]'s own grammar.
#[test]
fn a_memory_above_rule_parses_from_toml() {
    let rule: Rule = toml::from_str(
        r#"
on = "memory_above"
bytes = "512M"
sinks = ["oncall"]
"#,
    )
    .unwrap();
    assert_eq!(
        rule.when,
        Trigger::MemoryAbove {
            // Binary units: MemSize's grammar is MiB, not MB.
            bytes: MemSize::from_bytes(512 * 1024 * 1024),
        }
    );
}

#[test]
fn a_rule_s_debounce_override_parses_from_toml() {
    let rule: Rule = toml::from_str(
        r#"
on = "gave_up"
sinks = ["oncall"]
debounce = "10m"
"#,
    )
    .unwrap();
    assert_eq!(rule.debounce, UpDuration::from_millis(10 * 60_000));
}

#[test]
fn a_misspelled_trigger_field_is_refused_with_the_bad_key_named() {
    let err = toml::from_str::<Rule>(
        r#"
on = "restart_rate"
retsarts = 5
within = "2m"
sinks = ["oncall"]
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("retsarts"),
        "the error must name the misspelled key, not just fail: {err}"
    );
}

/// A bare `GaveUp` unit variant would skip field checking for the rest
/// of the map, missing exactly this typo.
#[test]
fn a_misspelled_field_next_to_gave_up_is_still_refused() {
    let err = toml::from_str::<Rule>(
        r#"
on = "gave_up"
sinks = ["oncall"]
debuonce = "10m"
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("debuonce"),
        "the error must name the misspelled key, not just fail: {err}"
    );
}

/// `sinsk` matches no field on `Rule` or `Trigger`, so it is simply
/// absent, and the error names the missing `sinks` field instead of
/// the typo.
#[test]
fn a_misspelled_sinks_field_is_refused_as_a_missing_field() {
    let err = toml::from_str::<Rule>(
        r#"
on = "gave_up"
sinsk = ["oncall"]
"#,
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("sinks"),
        "the error must name the missing field: {err}"
    );
}

#[test]
fn an_unknown_on_variant_is_refused_with_the_bad_value_named() {
    let err = toml::from_str::<Rule>(
        r#"
on = "gav_up"
sinks = ["oncall"]
"#,
    )
    .unwrap_err();
    let message = err.to_string();
    assert!(
        message.contains("gav_up"),
        "the error must name the bad value: {message}"
    );
    assert!(
        message.contains("gave_up"),
        "the error must also name a real variant, so a typo suggests its own fix: {message}"
    );
}

/// [`wire_spelling`] hand-lists the spellings instead of reading
/// `ProcessEventKind`'s own `Serialize`, which is what makes it free of
/// an allocation on the bus route. This pins every arm back against what
/// serde actually emits, so a typo in an arm fails here rather than as a
/// rule that silently stops firing days later.
///
/// The enum is `#[non_exhaustive]`, so this list is the only thing that
/// would notice a variant being added: a new one needs a line here as well
/// as an arm in `wire_spelling`.
#[test]
fn every_wire_spelling_matches_what_serde_emits() {
    let kinds = [
        ProcessEventKind::Start,
        ProcessEventKind::Online,
        ProcessEventKind::Exit,
        ProcessEventKind::Restart,
        ProcessEventKind::Reload,
        ProcessEventKind::Reloaded,
        ProcessEventKind::ReloadAbandoned,
        ProcessEventKind::Stop,
        ProcessEventKind::Delete,
        ProcessEventKind::Errored,
        ProcessEventKind::Unrecognized,
    ];
    for kind in kinds {
        let from_serde = serde_json::to_value(kind)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        assert_eq!(
            wire_spelling(kind),
            from_serde,
            "{kind:?} must spell itself the way its own Serialize does"
        );
    }
}

/// A subject bark has not seen yet still gets its state, which is what
/// keeps the debounce honest for a first firing: `subject_state` inserts
/// on a miss rather than assuming the key is there.
#[test]
fn a_subject_seen_for_the_first_time_gets_its_debounce_state() {
    let sinks = one_sink("ops");
    let mut rules = Rules::new(
        vec![Rule {
            when: Trigger::Event {
                kinds: vec!["exit".to_owned()],
            },
            sinks: vec!["ops".to_owned()],
            debounce: default_debounce(),
        }],
        &sinks,
    )
    .unwrap();
    // Two different sheep, neither in the map yet: both must be recorded
    // independently, and neither may fire twice inside the debounce.
    for name in ["web", "worker"] {
        assert_eq!(
            rules
                .on_event(&process_event(name, ProcessEventKind::Exit), 1_000)
                .len(),
            1,
            "{name} had never been seen and should fire once"
        );
        assert!(
            rules
                .on_event(&process_event(name, ProcessEventKind::Exit), 1_100)
                .is_empty(),
            "{name} is inside its own debounce now"
        );
    }
    // Past the debounce each fires again, which is only true if the state
    // the insert created is the state being read back.
    for name in ["web", "worker"] {
        assert_eq!(
            rules
                .on_event(&process_event(name, ProcessEventKind::Exit), 900_000)
                .len(),
            1,
            "{name} is past its debounce and should fire again"
        );
    }
}

// The log_match rule: parsing and refusal at startup.

fn log_match_rule(pattern: &str, stream: LogStream) -> Rule {
    Rule {
        when: Trigger::LogMatch {
            pattern: pattern.to_owned(),
            stream,
        },
        sinks: vec!["ops".to_owned()],
        debounce: default_debounce(),
    }
}

/// The exact shape `docs/dogs.md` and `web/src/pages/docs/dogs.astro`
/// publish as copy-pasteable.
#[test]
fn a_log_match_rule_parses_from_toml() {
    let doc: toml::Table = toml::from_str(
        r#"
[[bark.rules]]
on = "log_match"
pattern = "FATAL|panicked at"
stream = "err"
sinks = ["oncall"]
"#,
    )
    .unwrap();
    let rules: Vec<Rule> = doc["bark"]["rules"].clone().try_into().unwrap();
    let [rule] = rules.as_slice() else {
        panic!("one rule in the snippet: {rules:?}");
    };
    assert_eq!(
        rule.when,
        Trigger::LogMatch {
            pattern: "FATAL|panicked at".to_owned(),
            stream: LogStream::Err,
        }
    );
    assert_eq!(rule.sinks, vec!["oncall"]);
}

#[test]
fn a_log_match_rule_listens_to_both_streams_unless_told_otherwise() {
    let rule: Rule = toml::from_str(
        r#"
on = "log_match"
pattern = "FATAL"
sinks = ["oncall"]
"#,
    )
    .unwrap();
    assert_eq!(
        rule.when,
        Trigger::LogMatch {
            pattern: "FATAL".to_owned(),
            stream: LogStream::Both,
        }
    );
}

#[test]
fn a_misspelled_log_match_field_is_refused_with_the_bad_key_named() {
    let err = toml::from_str::<Rule>(
        r#"
on = "log_match"
pttaern = "FATAL"
sinks = ["oncall"]
"#,
    )
    .unwrap_err();
    assert!(err.to_string().contains("pttaern"), "{err}");
}

#[test]
fn a_log_match_rule_with_an_empty_pattern_is_refused() {
    let err = Rules::new(vec![log_match_rule("", LogStream::Both)], &one_sink("ops")).unwrap_err();
    assert!(
        matches!(err, RulesError::BadPattern { index: 0, .. }),
        "{err:?}"
    );
    assert!(err.to_string().contains("empty"), "{err}");
}

#[test]
fn a_log_match_rule_with_a_bad_pattern_is_refused_naming_the_rule() {
    let err = Rules::new(
        vec![rule_to("ops"), log_match_rule("(unclosed", LogStream::Both)],
        &one_sink("ops"),
    )
    .unwrap_err();
    assert!(
        matches!(err, RulesError::BadPattern { index: 1, .. }),
        "{err:?}"
    );
    assert!(
        err.to_string().starts_with("rule 1's log_match pattern"),
        "{err}"
    );
}

#[test]
fn an_oversized_pattern_is_refused() {
    // The pattern `selector_spec_oversized_regex_is_rejected` uses: it
    // overshoots 1 MiB by orders of magnitude, so a regex release that counts
    // compiled size loosely (1.8 did) still refuses it, and it needs no
    // Unicode feature the way a `\p{L}` repetition does.
    let huge = format!("(a{}){{10000}}", "|b".repeat(100_000));
    let err = Rules::new(
        vec![log_match_rule(&huge, LogStream::Both)],
        &one_sink("ops"),
    )
    .unwrap_err();
    let RulesError::BadPattern { reason, .. } = &err else {
        panic!("{err:?}");
    };
    assert!(reason.contains("size limit"), "{reason}");
}

// The log_match engine.

fn log_rules(pattern: &str, stream: LogStream) -> Rules {
    let mut rules = Rules::new(vec![log_match_rule(pattern, stream)], &one_sink("ops")).unwrap();
    rules.learn(&[named(7, "web"), named(8, "api")]);
    rules
}

fn named(id: u32, name: &str) -> ProcessInfo {
    ProcessInfo::builder(id, name, ProcStatus::Online)
        .pid(Some(4242))
        .build()
}

fn line_out(id: u32, line: &str) -> BusEvent {
    BusEvent::LogOut {
        id,
        line: line.to_owned(),
    }
}

fn line_err(id: u32, line: &str) -> BusEvent {
    BusEvent::LogErr {
        id,
        line: line.to_owned(),
    }
}

/// (rule's stream, which stream the line came on, whether it fires)
#[test]
fn a_line_fires_only_on_a_stream_the_rule_reads() {
    let cases = [
        (LogStream::Err, true, true),
        (LogStream::Err, false, false),
        (LogStream::Out, false, true),
        (LogStream::Out, true, false),
        (LogStream::Both, true, true),
        (LogStream::Both, false, true),
    ];
    for (stream, on_err, fires) in cases {
        let mut rules = log_rules("FATAL", stream);
        let event = if on_err {
            line_err(7, "FATAL: out of disk")
        } else {
            line_out(7, "FATAL: out of disk")
        };
        let fired = rules.on_event(&event, 1_000);
        assert_eq!(
            fired.len(),
            usize::from(fires),
            "rule on {stream:?}, line on {}",
            if on_err { "err" } else { "out" }
        );
    }
}

#[test]
fn a_matching_line_is_reported_for_its_sheep_as_a_log_match() {
    let mut rules = log_rules("FATAL|panicked at", LogStream::Err);
    let fired = rules.on_event(&line_err(7, "thread 'main' panicked at src/main.rs"), 1_000);
    assert_eq!(fired.len(), 1);
    assert_eq!(fired[0].bark.rule, "log_match");
    assert_eq!(fired[0].bark.subject, "web");
    assert_eq!(
        fired[0].bark.message,
        "web logged to stderr: thread 'main' panicked at src/main.rs"
    );
    assert_eq!(fired[0].sinks, vec!["ops"]);
}

#[test]
fn a_line_on_stdout_is_reported_as_stdout() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    let fired = rules.on_event(&line_out(7, "FATAL"), 1_000);
    assert_eq!(fired[0].bark.message, "web logged to stdout: FATAL");
}

#[test]
fn a_line_that_does_not_match_fires_nothing() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    assert!(rules.on_event(&line_err(7, "all good"), 1_000).is_empty());
}

#[test]
fn a_line_from_an_unknown_id_fires_nothing() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    assert!(rules.on_event(&line_err(99, "FATAL"), 1_000).is_empty());
}

#[test]
fn a_second_matching_line_inside_the_debounce_window_fires_nothing() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    assert_eq!(rules.on_event(&line_err(7, "FATAL one"), 1_000).len(), 1);
    assert!(rules.on_event(&line_err(7, "FATAL two"), 2_000).is_empty());
    let later = rules.on_event(
        &line_err(7, "FATAL three"),
        1_000 + default_debounce().as_millis(),
    );
    assert_eq!(later.len(), 1);
    assert!(later[0].bark.message.ends_with("FATAL three"));
}

#[test]
fn one_noisy_sheep_does_not_mute_another_for_log_match() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    assert_eq!(rules.on_event(&line_err(7, "FATAL"), 1_000).len(), 1);
    assert!(rules.on_event(&line_err(7, "FATAL"), 1_100).is_empty());
    let other = rules.on_event(&line_err(8, "FATAL"), 1_200);
    assert_eq!(other.len(), 1);
    assert_eq!(other[0].bark.subject, "api");
}

#[test]
fn a_matched_line_is_cut_to_the_message_cap() {
    let mut rules = log_rules("é", LogStream::Both);
    // 1,500 two-byte chars: byte 1,000 is a boundary, so shift the prefix to
    // put the cap in the middle of a char.
    let line = format!("x{}", "é".repeat(1_500));
    let fired = rules.on_event(&line_err(7, &line), 1_000);
    let message = &fired[0].bark.message;
    let prefix = "web logged to stderr: ";
    let cut = message.strip_prefix(prefix).expect("the prefix is intact");
    assert!(cut.len() <= 1_000, "{} bytes", cut.len());
    assert_eq!(
        cut.len(),
        999,
        "the cap lands inside a char, so it backs up one byte"
    );
    assert!(
        line.starts_with(cut),
        "a prefix of the line, whole chars only"
    );
}

#[test]
fn a_short_line_is_not_cut() {
    let mut rules = log_rules("é", LogStream::Both);
    let line = "é".repeat(500);
    let fired = rules.on_event(&line_err(7, &line), 1_000);
    assert_eq!(
        fired[0].bark.message,
        format!("web logged to stderr: {line}")
    );
}

#[test]
fn a_process_event_teaches_the_name_a_log_line_needs() {
    let mut rules = Rules::new(
        vec![log_match_rule("FATAL", LogStream::Both)],
        &one_sink("ops"),
    )
    .unwrap();
    assert!(rules.on_event(&line_err(1, "FATAL"), 1_000).is_empty());
    let _ = rules.on_event(&process_event("web", ProcessEventKind::Online), 1_000);
    assert_eq!(rules.on_event(&line_err(1, "FATAL"), 1_100).len(), 1);
}

#[test]
fn delete_forgets_a_sheeps_name() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    let _ = rules.on_event(
        &BusEvent::Process {
            event: ProcessEventKind::Delete,
            info: named(7, "web"),
            manually: true,
            at_ms: 0,
        },
        1_000,
    );
    assert!(rules.on_event(&line_err(7, "FATAL"), 1_100).is_empty());
    assert_eq!(
        rules.on_event(&line_err(8, "FATAL"), 1_100).len(),
        1,
        "api is untouched"
    );
}

#[test]
fn a_poll_replaces_the_name_map() {
    let mut rules = log_rules("FATAL", LogStream::Both);
    let _ = rules.on_poll(&[named(8, "api"), named(9, "worker")], 1_000);
    assert!(
        rules.on_event(&line_err(7, "FATAL"), 1_100).is_empty(),
        "web is gone"
    );
    let fired = rules.on_event(&line_err(9, "FATAL"), 1_100);
    assert_eq!(fired[0].bark.subject, "worker");
}

#[test]
fn a_rule_set_without_a_log_rule_keeps_no_names() {
    let mut rules = gave_up_rules();
    rules.learn(&[named(7, "web")]);
    let _ = rules.on_event(&process_event("web", ProcessEventKind::Online), 1_000);
    assert!(rules.names.is_empty());
}

#[test]
fn the_topics_follow_the_streams_the_rules_read() {
    let topics = |streams: &[LogStream]| {
        let rules = streams.iter().map(|s| log_match_rule("x", *s)).collect();
        Rules::new(rules, &one_sink("ops")).unwrap().log_topics()
    };
    assert!(topics(&[]).is_empty());
    assert_eq!(topics(&[LogStream::Err]), ["log.err"]);
    assert_eq!(topics(&[LogStream::Out]), ["log.out"]);
    assert_eq!(topics(&[LogStream::Both]), ["log.out", "log.err"]);
    assert_eq!(
        topics(&[LogStream::Out, LogStream::Err]),
        ["log.out", "log.err"]
    );
}

#[test]
fn a_reload_carries_the_names_into_the_new_rules() {
    let mut old = log_rules("FATAL", LogStream::Both);
    let mut next = Rules::new(
        vec![log_match_rule("ERROR", LogStream::Both)],
        &one_sink("ops"),
    )
    .unwrap();
    next.take_names_from(&mut old);
    assert_eq!(next.on_event(&line_err(7, "ERROR"), 1_000).len(), 1);
}
