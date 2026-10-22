use super::config::BarkConfig;
use super::rules::Firing;
use super::sinks::Sink;
use futures_util::future::join_all;
use shep_core::barks::{self, SinkOutcome};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Everything a delivery needs, so the five values that travel together
/// through [`reconcile`](super::event_loop::reconcile), [`spawn_firings`] and
/// [`deliver_and_record`] travel as one.
///
/// `Clone` is what [`spawn_firings`] hands each spawned task: three
/// [`Arc`] bumps and two copies.
///
/// `Debug` is safe despite `sinks` holding webhook URLs, which are bearer
/// credentials: [`Sink`]'s own `Debug` redacts them.
#[derive(Debug, Clone)]
pub(super) struct Delivery {
    /// Every configured sink by name, for resolving a firing's own list.
    pub(super) sinks: Arc<BTreeMap<String, Sink>>,
    /// Serializes this process's appends to `barks_path`.
    pub(super) append_lock: Arc<Mutex<()>>,
    /// Where the bark trail is written.
    pub(super) barks_path: Arc<PathBuf>,
    /// How long one sink delivery may take.
    pub(super) sink_timeout: Duration,
    /// The trail's size ceiling.
    pub(super) max_bytes: u64,
}

impl Delivery {
    /// A delivery to `config`'s sinks, writing its trail to `barks_path`.
    pub(super) fn new(config: &BarkConfig, barks_path: &Path) -> Self {
        Self {
            sinks: Arc::new(config.sinks.clone()),
            append_lock: Arc::new(Mutex::new(())),
            barks_path: Arc::new(barks_path.to_path_buf()),
            sink_timeout: config.sink_timeout.as_duration(),
            max_bytes: config.history_bytes,
        }
    }

    /// Takes what a reloaded `config` changes, in place. The append lock
    /// and the trail's path outlive every reload.
    pub(super) fn reconfigure(&mut self, config: &BarkConfig) {
        self.sinks = Arc::new(config.sinks.clone());
        self.sink_timeout = config.sink_timeout.as_duration();
        self.max_bytes = config.history_bytes;
    }
}

/// Spawns one delivery task per firing, so
/// [`run_loop`](super::event_loop::run_loop)'s own `select!` returns to
/// reading the next event immediately rather than waiting on any of them.
pub(super) fn spawn_firings(firings: Vec<Firing>, delivery: &Delivery) {
    for firing in firings {
        let delivery = delivery.clone();
        tokio::spawn(async move {
            deliver_and_record(firing, &delivery).await;
        });
    }
}

/// Delivers `firing` to each of its named sinks, then writes the resulting
/// [`shep_core::barks::Bark`] to `barks_path`.
///
/// The sinks are driven together rather than in turn: a dead endpoint's
/// timeout runs beside the healthy sinks' deliveries instead of ahead of
/// them, so one unreachable webhook no longer holds up an alert the
/// others could have carried. Each delivery keeps its own
/// [`Delivery::sink_timeout`] and its own [`SinkOutcome`], and `join_all`
/// hands the outcomes back in `firing.sinks`' order, which is the order
/// the trail has always recorded them in.
///
/// The bark is written after delivery, since a [`Firing`]'s
/// [`shep_core::barks::Bark::sinks`] is empty until each sink has been tried.
/// Written even when every sink refused it: the local trail is what an
/// operator reads when the page never arrived.
///
/// [`Delivery::append_lock`] covers only the [`barks::append`] call, a
/// read-modify-rename against one file that several of these run at once.
/// It does not replace `append`'s own cross-process `flock(2)`.
///
/// The append runs on `tokio`'s blocking pool rather than inline. Measured
/// 2026-10-13 on an Apple Silicon mac (macOS 26.2) with
/// `benches/benches/bark_append.rs` at commit `55a769f0`: a ring already
/// at [`barks::DEFAULT_MAX_BYTES`] costs a median of 26.5 ms per append
/// (slowest sample 31.4 ms), an empty ring a median of 8.0 ms (slowest
/// 9.9 ms). Both sit far over the 1 ms that would have left the call
/// inline, and the empty-ring figure says most of it is the two `fsync`s
/// rather than the ring-sized rewrite. Inline on a one-worker runtime that
/// is 27 ms of reading no bus frames. The [`Delivery::append_lock`] guard
/// is still held across the `.await`, so this process's appends queue on
/// one lock rather than parking blocking-pool threads on `flock(2)`.
async fn deliver_and_record(firing: Firing, delivery: &Delivery) {
    let mut bark = firing.bark;
    let outcomes = join_all(firing.sinks.iter().map(|name| async {
        match delivery.sinks.get(name) {
            Some(sink) => match super::sinks::deliver(sink, &bark, delivery.sink_timeout).await {
                Ok(()) => SinkOutcome {
                    sink: name.clone(),
                    error: None,
                },
                Err(err) => SinkOutcome {
                    sink: name.clone(),
                    error: Some(err.to_string()),
                },
            },
            // Unreachable: `Rules::new` refuses a rule routing to a sink
            // `[bark.sinks]` does not define. Recorded rather than
            // panicked on.
            None => SinkOutcome {
                sink: name.clone(),
                error: Some("sink not configured".to_owned()),
            },
        }
    }))
    .await;
    bark.sinks = outcomes;

    // Held across the `.await` below: the guard, not `flock(2)`, is where
    // this process's appends queue, so at most one blocking-pool thread
    // ever waits on the cross-process lock.
    let _guard = delivery.append_lock.lock().await;
    let barks_path = Arc::clone(&delivery.barks_path);
    let max_bytes = delivery.max_bytes;
    match tokio::task::spawn_blocking(move || barks::append(&barks_path, &bark, max_bytes)).await {
        Ok(Ok(())) => {}
        Ok(Err(err)) => eprintln!("shep dog bark: could not record a fired bark: {err}"),
        // A panic in the pool thread, or a shutdown pool: the bark is lost
        // either way, so it gets the same report as a failed append.
        Err(err) => eprintln!("shep dog bark: could not record a fired bark: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use shep_core::barks::Bark;
    use shep_core::values::UpDuration;

    use super::super::testing::*;
    use super::*;

    /// A `gave_up` firing for `web`, routed to `sinks`.
    fn gave_up_firing(sinks: &[&str]) -> Firing {
        Firing {
            bark: Bark {
                at_ms: 1_000,
                rule: "gave_up".to_owned(),
                subject: "web".to_owned(),
                message: "web gave up: restart budget exhausted".to_owned(),
                sinks: Vec::new(),
            },
            sinks: sinks.iter().map(|&name| name.to_owned()).collect(),
        }
    }

    /// Drives `deliver_and_record` directly rather than through
    /// `run_loop`: the property belongs to that function, and the loop's
    /// event plumbing would need a second synchronization mechanism to
    /// know when a failed delivery finished.
    #[tokio::test]
    async fn a_bark_is_recorded_even_when_every_sink_refuses_it() {
        let (addr, _captured) = one_shot_sink(500, "refused").await;
        let dir = tempfile::tempdir().unwrap();
        let barks_path = dir.path().join("barks.jsonl");

        let mut sinks = BTreeMap::new();
        sinks.insert("ops".to_owned(), json_sink(format!("http://{addr}/hook")));
        let firing = gave_up_firing(&["ops"]);

        let delivery = Delivery {
            sinks: Arc::new(sinks),
            append_lock: Arc::new(Mutex::new(())),
            barks_path: Arc::new(barks_path.clone()),
            sink_timeout: Duration::from_secs(5),
            max_bytes: barks::DEFAULT_MAX_BYTES,
        };
        // Wider than `sink_timeout` by a margin a refusal from localhost
        // never needs, so only a regression into a hang reaches it.
        tokio::time::timeout(
            Duration::from_secs(30),
            deliver_and_record(firing, &delivery),
        )
        .await
        .expect("the delivery outlived its own sink timeout");

        let recorded = shep_core::barks::read(&barks_path).unwrap();
        assert_eq!(
            recorded.len(),
            1,
            "a refused delivery must still be recorded"
        );
        assert_eq!(recorded[0].subject, "web");
        assert!(
            recorded[0].sinks[0].error.is_some(),
            "the 500 must be recorded as a failed delivery, not silently dropped"
        );
    }

    /// A dead endpoint's timeout must run beside the healthy sinks, not
    /// ahead of them. Asserted as an order rather than a duration, the
    /// way `slow_sink`'s own doc suggests, and read off events rather
    /// than off a clock: the live sink's request has to arrive while the
    /// dead delivery is still hanging, and the trail has to be empty at
    /// the moment it lands. Driven in turn, the live sink is not
    /// contacted until that delivery has already ended, so the order is
    /// what fails.
    ///
    /// Every wait below is a failure guard, none of them a budget a
    /// loaded machine has to beat, and the test does not pay its own
    /// `sink_timeout`: the dead sink is released once the order is
    /// proven, so the delivery ends on the closed connection.
    #[tokio::test]
    async fn a_dead_sink_does_not_delay_the_live_ones_delivery() {
        let (dead_addr, dead_connected, dead_release) = slow_sink_with_release().await;
        let (live_addr, live_request) = one_shot_sink(200, "ok").await;
        let dir = tempfile::tempdir().unwrap();
        let barks_path = dir.path().join("barks.jsonl");

        let mut sinks = BTreeMap::new();
        sinks.insert(
            "dead".to_owned(),
            json_sink(format!("http://{dead_addr}/hook")),
        );
        sinks.insert(
            "live".to_owned(),
            json_sink(format!("http://{live_addr}/hook")),
        );
        let firing = gave_up_firing(&["dead", "live"]);

        let delivery = Delivery {
            sinks: Arc::new(sinks),
            append_lock: Arc::new(Mutex::new(())),
            barks_path: Arc::new(barks_path.clone()),
            // Ten minutes, as `a_slow_sink_never_stalls_the_loop` holds
            // it, and well above the guards below: the claim is that the
            // live request landed while the dead delivery still had this
            // budget unspent. Nothing waits it out, it is released.
            sink_timeout: Duration::from_secs(600),
            max_bytes: barks::DEFAULT_MAX_BYTES,
        };
        let recorded = tokio::spawn(async move { deliver_and_record(firing, &delivery).await });

        // Each wait is a guard against a delivery that never ends, not a
        // budget a loaded machine has to beat.
        let guard = Duration::from_secs(30);

        tokio::time::timeout(guard, dead_connected)
            .await
            .expect("the dead sink must be contacted at all")
            .expect("the dead sink's accept signal must arrive");

        // The live sink is reached while the dead one is still hanging:
        // nothing has released it, so its delivery cannot have ended.
        let request = tokio::time::timeout(guard, live_request)
            .await
            .expect(
                "the live sink must hear about the alert while the dead one still hangs; \
                 driven in turn it would have waited out the dead sink's timeout first",
            )
            .expect("the live sink must have captured its request");
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/hook");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&request.body).unwrap()["subject"],
            "web",
            "the live sink must have been delivered the alert, not merely contacted"
        );

        // The same order from the other side, with no wait at all: the
        // trail is written only once every sink has been tried, so an
        // entry here would mean the dead sink had produced its outcome
        // before the live one was contacted.
        assert!(
            barks::read(&barks_path).unwrap().is_empty(),
            "no bark may be recorded while the dead sink is still hanging"
        );

        // End the hang rather than wait it out: the release closes the
        // dead connection, and the delivery finishes on that.
        let _ = dead_release.send(());

        tokio::time::timeout(guard, recorded)
            .await
            .expect("the delivery must end once the dead sink lets go, not outlive its timeout")
            .expect("the delivery task must not panic");
        let barks = barks::read(&barks_path).unwrap();
        assert_eq!(barks.len(), 1);
        let outcomes = &barks[0].sinks;
        assert_eq!(outcomes.len(), 2, "both sinks must be recorded");
        assert_eq!(
            outcomes[0].sink, "dead",
            "the trail keeps the firing's order"
        );
        assert!(
            outcomes[0].error.is_some(),
            "the dead sink's failed delivery is recorded as a failure, not dropped"
        );
        assert_eq!(outcomes[1].sink, "live");
        assert!(
            outcomes[1].error.is_none(),
            "the live sink must have been delivered to: {:?}",
            outcomes[1]
        );
    }

    #[test]
    fn a_reconfigure_takes_the_new_settings_and_keeps_the_trail() {
        let dir = tempfile::tempdir().unwrap();
        let barks_path = dir.path().join("barks.jsonl");
        let before = config_with_sink("127.0.0.1:1".parse().unwrap());
        let mut delivery = Delivery::new(&before, &barks_path);
        let lock = Arc::clone(&delivery.append_lock);

        let mut after = config_with_sink("127.0.0.1:2".parse().unwrap());
        after.sink_timeout = UpDuration::from_millis(1_234);
        after.history_bytes = 4_096;
        delivery.reconfigure(&after);

        assert_eq!(*delivery.sinks, after.sinks);
        assert_eq!(delivery.sink_timeout, Duration::from_millis(1_234));
        assert_eq!(delivery.max_bytes, 4_096);
        assert!(
            Arc::ptr_eq(&delivery.append_lock, &lock),
            "a second lock would let two appends race"
        );
        assert_eq!(*delivery.barks_path, barks_path);
    }

    /// Tests that wait out a real wall-clock hold of the ring file's lock.
    /// The elapsed time is the point: a run that parked its tokio worker
    /// would pass every assertion but this file's reason for existing.
    mod slow {
        use super::*;
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// fails if the ring write parks the tokio worker instead of
        /// waiting on the blocking pool.
        ///
        /// A plain OS thread holds the [`shep_core::file_lock::FileLock`]
        /// `append` needs for 300 ms, so the append can only finish after
        /// that hold. On a `current_thread` runtime with the call inline,
        /// the single worker is parked for the whole hold and a task
        /// ticking every 10 ms gets 0 or 1 ticks; on the blocking pool the
        /// worker keeps running and gets roughly 30.
        #[tokio::test(flavor = "current_thread")]
        async fn a_ring_write_waiting_on_another_writer_leaves_the_worker_running() {
            let dir = tempfile::tempdir().unwrap();
            let barks_path = dir.path().join("barks.jsonl");

            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
            let lock_path = barks_path.clone();
            let holder = std::thread::spawn(move || {
                let guard = shep_core::file_lock::FileLock::acquire(&lock_path).unwrap();
                locked_tx.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(300));
                drop(guard);
            });
            // The hold must be real before the delivery starts, or the
            // append wins the flock uncontended and the test measures a
            // delivery nobody blocked. The sender drops only when the
            // holder's closure ends, so a disconnect before the message
            // means the holder died before reporting the hold: take its
            // panic rather than spin on a hold that will never come.
            loop {
                match locked_rx.try_recv() {
                    Ok(()) => break,
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        holder
                            .join()
                            .expect("the lock holder died before it reported the hold");
                        unreachable!("a holder that ends cleanly sends before it disconnects");
                    }
                }
            }

            let delivery = Delivery {
                sinks: Arc::new(BTreeMap::new()),
                append_lock: Arc::new(Mutex::new(())),
                barks_path: Arc::new(barks_path.clone()),
                sink_timeout: Duration::from_secs(5),
                max_bytes: barks::DEFAULT_MAX_BYTES,
            };
            // No sinks: everything left in `deliver_and_record` is the
            // ring write.
            let firing = gave_up_firing(&[]);

            let ticks = Arc::new(AtomicUsize::new(0));
            let ticking = {
                let ticks = Arc::clone(&ticks);
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        ticks.fetch_add(1, Ordering::Relaxed);
                    }
                })
            };

            deliver_and_record(firing, &delivery).await;
            ticking.abort();
            holder.join().unwrap();

            let counted = ticks.load(Ordering::Relaxed);
            assert!(
                counted >= 5,
                "the worker ticked {counted} times during the 300 ms hold; \
                 inline, the single worker is parked for the whole hold and \
                 the count is 0 or 1"
            );

            let recorded = barks::read(&barks_path).unwrap();
            assert_eq!(recorded.len(), 1, "the bark lands after the wait");
            assert_eq!(recorded[0].subject, "web");
        }
    }
}
