//! TEMPORARY DIAGNOSTIC. Delete this file and its `mod` line once read.
//!
//! `a_respawn_that_cannot_spawn_stops_the_name_groups_cron_worker` failed once
//! on `windows-latest` and passes on every local run. The CI log shows only the
//! test name, not the panic payload, so this replays that case's body at rate
//! and carries the payload of the first failure into the panic message.
//!
//! It sleeps in real time before it panics so that it is the last test to
//! finish in its run: nextest prints a failing test's captured output where the
//! test completes, and only the last one lands inside a log tail.

use super::*;
use std::sync::Mutex;
use std::sync::mpsc;

const ROUNDS: usize = 2_000;
const THREADS: usize = 16;
/// Longer than any other test in the run (3.3s worst measured), short enough
/// to stay under nextest's slow-test termination.
const FINISH_LAST: std::time::Duration = std::time::Duration::from_secs(60);

/// The failing case's body, copied rather than shared so the probe keeps its
/// own settle loop.
async fn body() {
    let clock = Arc::new(TestClock::starting_at(dt("2026-01-01T00:00:00Z")));
    let h = harness_with_extras(vec![ProcScript::never_exits()], |reports| Extras {
        clock: Arc::clone(&clock) as Arc<dyn Clock>,
        enforcer: Arc::new(RecordingEnforcer::default()),
        lamb_sweep: idle_sweep(),
        max_cron_sleep: DEFAULT_MAX_CRON_SLEEP,
        reports,
        stats: idle_stats(),
    });
    let mut rx = h.ctx.events.subscribe();
    h.ctx
        .supervisor
        .start(vec![app_with("web", |app| {
            app.cron_restart = Some("0 * * * *".to_string());
        })])
        .await
        .unwrap();

    cross_one_hour().await;
    for _ in 0..200 {
        tokio::task::yield_now().await;
        if h.ctx
            .supervisor
            .list()
            .await
            .iter()
            .any(|info| info.id == 0 && info.status == ProcStatus::Errored)
        {
            break;
        }
    }
    let reads_after_error = clock.reads();
    assert_no_restart_within(&mut rx, "web", PAST_THE_NEXT_OCCURRENCE).await;
    assert_eq!(
        clock.reads(),
        reads_after_error,
        "an errored sheep's cron worker must stop reading the clock"
    );
}

/// One round on its own current-thread paused runtime, which is what
/// `#[tokio::test(start_paused = true)]` builds.
fn one_round() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("probe runtime");
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        runtime.block_on(body());
    }));
    match outcome {
        Ok(()) => Ok(()),
        Err(payload) => {
            let text = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_string());
            Err(text)
        }
    }
}

#[test]
fn probe_the_cron_worker_case_at_rate() {
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_for_hook = Arc::clone(&seen);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Ok(mut log) = seen_for_hook.lock() {
            log.push(format!("{info}"));
        }
    }));

    let (tx, rx) = mpsc::channel::<(usize, String)>();
    let mut workers = Vec::new();
    for thread in 0..THREADS {
        let tx = tx.clone();
        workers.push(std::thread::spawn(move || {
            let mut round = thread;
            while round < ROUNDS {
                if let Err(payload) = one_round() {
                    let _ = tx.send((round, payload));
                }
                round += THREADS;
            }
        }));
    }
    drop(tx);
    let mut failures: Vec<(usize, String)> = rx.iter().collect();
    for worker in workers {
        let _ = worker.join();
    }
    std::panic::set_hook(previous);

    failures.sort();
    let distinct = {
        let mut lines: Vec<String> = failures.iter().map(|(_, p)| p.clone()).collect();
        lines.sort();
        lines.dedup();
        lines
    };
    let hook_lines: Vec<String> = seen.lock().unwrap().iter().take(4).cloned().collect();

    std::thread::sleep(FINISH_LAST);

    panic!(
        "PROBE628 os={} rounds={ROUNDS} threads={THREADS} failures={} distinct={}\n\
         first payloads:\n{}\n\
         hook payloads:\n{}",
        std::env::consts::OS,
        failures.len(),
        distinct.len(),
        if distinct.is_empty() {
            "none".to_string()
        } else {
            distinct
                .iter()
                .take(4)
                .map(|p| format!("  {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
        if hook_lines.is_empty() {
            "none".to_string()
        } else {
            hook_lines
                .iter()
                .map(|p| format!("  {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    );
}
