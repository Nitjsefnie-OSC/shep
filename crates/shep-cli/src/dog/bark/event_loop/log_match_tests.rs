//! The loop's half of the `log_match` rule: resolving a sheep from a log
//! frame, and keeping the subscription in step with the rules.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use shep_client::{LinkLost, RequestError};
use shep_core::protocol::{ProcessEventKind, ProcessInfo};
use shep_core::status::ProcStatus;
use shep_core::values::UpDuration;
use tokio::sync::broadcast;

use super::super::rules::{LogStream, Rule, Trigger};
use super::*;
use crate::dog::bark::testing::*;

/// A sink nothing listens on.
const DEAD: &str = "127.0.0.1:1";

const BASE: [&str; 2] = ["process.*", "config.dog.bark"];

type Log = Arc<Mutex<Vec<String>>>;

fn topics_of(extra: &[&str]) -> Vec<String> {
    BASE.iter().chain(extra).map(|t| (*t).to_owned()).collect()
}

/// A broadcast receiver that holds a topic list and writes every
/// `set_topics` it is given into `log`.
struct TopicSource {
    rx: broadcast::Receiver<BusEvent>,
    topics: Vec<String>,
    log: Log,
}

impl EventSource for TopicSource {
    async fn next(&mut self) -> Option<Result<BusEvent, u64>> {
        EventSource::next(&mut self.rx).await
    }

    fn topics(&self) -> &[String] {
        &self.topics
    }

    async fn set_topics(&mut self, topics: Vec<String>) -> Result<(), Resubscribe> {
        self.log
            .lock()
            .unwrap()
            .push(format!("set_topics {}", topics.join(",")));
        self.topics = topics;
        Ok(())
    }

    async fn resubscribe(&mut self) -> Result<(), Resubscribe> {
        EventSource::resubscribe(&mut self.rx).await
    }
}

/// A [`FlockSource`] that writes each poll into the same `log`.
struct LoggedFlock {
    answer: Vec<ProcessInfo>,
    log: Log,
}

impl FlockSource for LoggedFlock {
    async fn flock(&self) -> Result<Vec<ProcessInfo>, RequestError> {
        self.log.lock().unwrap().push("poll".to_owned());
        Ok(self.answer.clone())
    }
}

fn log_match_rule(stream: LogStream) -> Rule {
    Rule {
        when: Trigger::LogMatch {
            pattern: "FATAL".to_owned(),
            stream,
        },
        sinks: vec!["ops".to_owned()],
        debounce: UpDuration::from_millis(5 * 60_000),
    }
}

fn rules_of(rules: Vec<Rule>) -> Rules {
    let mut sinks = BTreeMap::new();
    sinks.insert(
        "ops".to_owned(),
        json_sink("http://127.0.0.1:1/hook".to_owned()),
    );
    Rules::new(rules, &sinks).unwrap()
}

fn line_err(id: u32, line: &str) -> BusEvent {
    BusEvent::LogErr {
        id,
        line: line.to_owned(),
    }
}

/// A `[bark]` section with the sink at `addr` and `rules` as written.
fn section(addr: &str, rules: &str) -> String {
    format!("[sinks.ops]\nkind = \"json\"\nurl = \"http://{addr}/hook\"\n{rules}")
}

const ERR_RULE: &str =
    "[[rules]]\non = \"log_match\"\npattern = \"FATAL\"\nstream = \"err\"\nsinks = [\"ops\"]\n";

struct Harness {
    tx: broadcast::Sender<BusEvent>,
    log: Log,
    barks_path: std::path::PathBuf,
    handle: tokio::task::JoinHandle<ExitCode>,
    _dir: tempfile::TempDir,
}

/// Starts the loop on a [`TopicSource`] subscribed to `topics`, over a flock
/// answering `flock`, with `rules` and a config source answering `section`.
fn start(
    rules: Rules,
    topics: Vec<String>,
    flock: Vec<ProcessInfo>,
    addr: &str,
    section: String,
) -> Harness {
    let (tx, rx) = broadcast::channel(16);
    let log = Log::default();
    let dir = tempfile::tempdir().unwrap();
    let barks_path = dir.path().join("barks.jsonl");
    let handle = tokio::spawn(run_loop(
        TopicSource {
            rx,
            topics,
            log: Arc::clone(&log),
        },
        LoggedFlock {
            answer: flock,
            log: Arc::clone(&log),
        },
        rules,
        &config_with_sink(addr.parse().unwrap()),
        &barks_path,
        ScriptedConfig::answering(section),
        Stop::never(),
    ));
    Harness {
        tx,
        log,
        barks_path,
        handle,
        _dir: dir,
    }
}

impl Harness {
    fn log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    /// Waits until the loop has written `count` log entries.
    async fn log_of(&self, count: usize) -> Vec<String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let log = self.log();
                if log.len() >= count {
                    break log;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the loop must reach that many calls")
    }

    fn reload(&self) {
        self.tx
            .send(BusEvent::DogConfigChanged {
                dog: "bark".to_owned(),
            })
            .unwrap();
    }
}

async fn recorded(barks_path: &std::path::Path) -> Vec<shep_core::barks::Bark> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let records = shep_core::barks::read(barks_path).unwrap();
            if !records.is_empty() {
                break records;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the delivered bark must be recorded")
}

/// Fails if a log frame cannot find its sheep from the `Process` frame
/// that introduced it, or if the bark is not recorded. The sink is
/// unreachable: a bark is recorded whether or not its delivery succeeds.
#[tokio::test]
async fn a_log_line_from_a_sheep_seen_on_the_bus_delivers_a_bark() {
    let harness = start(
        rules_of(vec![log_match_rule(LogStream::Err)]),
        topics_of(&["log.err"]),
        Vec::new(),
        DEAD,
        String::new(),
    );
    harness
        .tx
        .send(process_event("web", ProcessEventKind::Online))
        .unwrap();
    harness.tx.send(line_err(1, "FATAL: out of disk")).unwrap();

    let records = recorded(&harness.barks_path).await;
    assert_eq!(records[0].rule, "log_match");
    assert_eq!(records[0].subject, "web");
    assert_eq!(
        records[0].message,
        "web logged to stderr: FATAL: out of disk"
    );
    harness.handle.abort();
}

/// Fails if the map starts empty: the log frame is the first thing the
/// loop sees, so only the startup read of the flock can name sheep 1.
#[tokio::test]
async fn a_log_line_before_any_poll_or_process_frame_still_resolves_its_sheep() {
    let harness = start(
        rules_of(vec![log_match_rule(LogStream::Err)]),
        topics_of(&["log.err"]),
        vec![base_info("web", ProcStatus::Online, 0)],
        DEAD,
        String::new(),
    );
    harness.tx.send(line_err(1, "FATAL")).unwrap();

    let records = recorded(&harness.barks_path).await;
    assert_eq!(records[0].subject, "web");
    harness.handle.abort();
}

/// Fails if priming reconciles: an `Errored` sheep already in the flock
/// would fire `gave_up` at startup, earlier than it does without a log
/// rule, and a second bark would sit beside the line's.
#[tokio::test]
async fn priming_the_names_does_not_evaluate_any_rule() {
    let gave_up = Rule {
        when: Trigger::GaveUp {},
        sinks: vec!["ops".to_owned()],
        debounce: UpDuration::from_millis(5 * 60_000),
    };
    let harness = start(
        rules_of(vec![gave_up, log_match_rule(LogStream::Err)]),
        topics_of(&["log.err"]),
        vec![errored_info("web", 3)],
        DEAD,
        String::new(),
    );
    harness.tx.send(line_err(1, "FATAL")).unwrap();

    let _ = recorded(&harness.barks_path).await;
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
    let rules: Vec<_> = shep_core::barks::read(&harness.barks_path)
        .unwrap()
        .into_iter()
        .map(|bark| bark.rule)
        .collect();
    assert_eq!(rules, ["log_match"]);
    harness.handle.abort();
}

/// Fails if a rule set with no log rule reads the flock at startup.
#[tokio::test]
async fn without_a_log_rule_the_flock_is_not_read_at_startup() {
    let harness = start(
        gave_up_rules(),
        topics_of(&[]),
        Vec::new(),
        DEAD,
        String::new(),
    );
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.log(), Vec::<String>::new());
    harness.handle.abort();
}

/// Fails if a reload that adds a log rule does not widen the subscription
/// and then poll, since frames unread on the old stream are gone.
#[tokio::test]
async fn a_reload_that_adds_a_log_rule_subscribes_to_the_log_topic_then_polls() {
    let harness = start(
        gave_up_rules(),
        topics_of(&[]),
        Vec::new(),
        DEAD,
        section(DEAD, ERR_RULE),
    );
    harness.reload();

    let log = harness.log_of(2).await;
    assert_eq!(
        log,
        ["set_topics process.*,config.dog.bark,log.err", "poll"]
    );
    harness.handle.abort();
}

/// Fails if a reload that drops the last log rule keeps asking for lines.
#[tokio::test]
async fn a_reload_that_removes_the_log_rule_drops_the_log_topic_then_polls() {
    let harness = start(
        rules_of(vec![log_match_rule(LogStream::Err)]),
        topics_of(&["log.err"]),
        Vec::new(),
        DEAD,
        section(DEAD, ""),
    );
    harness.reload();

    let log = harness.log_of(3).await;
    assert_eq!(
        log,
        ["poll", "set_topics process.*,config.dog.bark", "poll"],
        "the first poll is the startup read of the names"
    );
    harness.handle.abort();
}

/// Fails if a reload that leaves the topic set alone resubscribes or polls.
/// The bark is also the proof the reload is over, and that the names
/// survived it: the sheep was introduced before the reload.
#[tokio::test]
async fn a_reload_that_leaves_the_topics_alone_calls_nothing() {
    let harness = start(
        rules_of(vec![log_match_rule(LogStream::Err)]),
        topics_of(&["log.err"]),
        Vec::new(),
        DEAD,
        section(DEAD, ERR_RULE),
    );
    harness
        .tx
        .send(process_event("web", ProcessEventKind::Online))
        .unwrap();
    harness.reload();
    harness.tx.send(line_err(1, "FATAL")).unwrap();

    let records = recorded(&harness.barks_path).await;
    assert_eq!(
        records[0].subject, "web",
        "the first line after a reload must still resolve its sheep"
    );
    assert_eq!(
        harness.log(),
        ["poll"],
        "only the startup read of the names"
    );
    harness.handle.abort();
}

/// Fails if a failed `set_topics` is shrugged off: the dog would keep
/// running on a subscription that no longer matches its rules.
#[tokio::test]
async fn a_failed_set_topics_ends_the_dog() {
    struct Refusing(broadcast::Receiver<BusEvent>);
    impl EventSource for Refusing {
        async fn next(&mut self) -> Option<Result<BusEvent, u64>> {
            EventSource::next(&mut self.0).await
        }
        fn topics(&self) -> &[String] {
            &[]
        }
        async fn set_topics(&mut self, _topics: Vec<String>) -> Result<(), Resubscribe> {
            Err(Resubscribe::Lost(LinkLost::Budget {
                waited: Duration::from_secs(5),
            }))
        }
        async fn resubscribe(&mut self) -> Result<(), Resubscribe> {
            EventSource::resubscribe(&mut self.0).await
        }
    }

    let (tx, rx) = broadcast::channel(16);
    let dir = tempfile::tempdir().unwrap();
    let handle = tokio::spawn(run_loop(
        Refusing(rx),
        ScriptedFlock::answering(Vec::new()),
        gave_up_rules(),
        &config_with_sink(DEAD.parse().unwrap()),
        &dir.path().join("barks.jsonl"),
        ScriptedConfig::answering(section(DEAD, ERR_RULE)),
        Stop::never(),
    ));
    tx.send(BusEvent::DogConfigChanged {
        dog: "bark".to_owned(),
    })
    .unwrap();

    let code = tokio::time::timeout(Duration::from_secs(5), handle)
        .await
        .expect("a dog that cannot hold its subscription must end")
        .unwrap();
    assert_eq!(code, ExitCode::DaemonUnreachable);
}
