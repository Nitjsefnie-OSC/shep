//! The `log_match` rule's own half: its pattern and stream parsed out of
//! TOML, a pattern that cannot work refused at startup, and the engine that
//! turns a log frame into a firing.

use super::*;

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
