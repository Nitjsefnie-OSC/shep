//! The two threads behind a live channel: the writer drains the outbox
//! onto the transport, and the reader answers what the shepherd sends.
//!
//! Free functions over `BufRead` and `Write`, so a test drives either one
//! over a buffer with no thread or socket.

use std::io::{BufRead, Write};
use std::sync::RwLock;

use crate::dispatch::{Dispatch, Outcome, run};
use crate::outbox::Outbox;
use crate::{ChannelError, session};

/// What to tell an author whose app was asked to stop and registered nothing.
const UNHANDLED_SHUTDOWN_ADVICE: &str = "the shepherd sent shutdown and no on_shutdown handler is registered. This \
     process will be killed when kill_timeout expires. Register one to stop gracefully.";

/// Drives the writer side: drains `outbox` and writes what it took, a
/// burst at a time. Stops once the transport fails or the outbox closes.
pub(crate) fn writer_loop<W: Write>(writer: &mut W, outbox: &Outbox) {
    let mut batch = Vec::new();
    let mut line = Vec::new();
    while outbox.take_batch(&mut batch) {
        if session::write_messages(writer, &batch, &mut line).is_err() {
            break;
        }
        outbox.wrote(batch.len());
    }
    // `stop`, not `close`: a failed write leaves its message unwritten,
    // and a `flush` waiting on it has to hear that rather than time out.
    outbox.stop();
}

/// Drives the reader side: reads one message, resolves it against
/// `dispatch`, and runs the result. The registry's lock is dropped before
/// that run, per `dispatch`'s own module doc.
///
/// `warn` is injected so a test can assert what was said and how
/// often. This pins the once-per-loop malformed-frame latch.
pub(crate) fn reader_loop<R: BufRead>(
    reader: &mut R,
    outbox: &Outbox,
    dispatch: &RwLock<Dispatch>,
    warn: &dyn Fn(&str),
) {
    let mut warned_malformed = false;
    let mut line = Vec::new();
    loop {
        match session::read_message(reader, &mut line) {
            Ok(Some(message)) => {
                let resolved = dispatch
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .resolve(message);
                let outcome = run(resolved);
                match outcome {
                    Outcome::Reply(reply) => {
                        if outbox.push_blocking(reply).is_err() {
                            break;
                        }
                    }
                    Outcome::Handled => {}
                    Outcome::UnhandledShutdown => warn(UNHANDLED_SHUTDOWN_ADVICE),
                    Outcome::ShutdownFailed(message) => {
                        warn(&format!("shutdown handler panicked: {message}"));
                    }
                }
            }
            Err(ChannelError::Malformed(message)) => {
                if !warned_malformed {
                    warned_malformed = true;
                    warn(&format!("malformed frame from the shepherd: {message}"));
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
    outbox.close();
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::Arc;

    use super::*;
    use crate::ChildMessage;

    /// This warning is the only thing standing between a missing handler
    /// and a `kill_timeout` kill.
    #[test]
    fn the_unhandled_shutdown_warning_names_the_method_to_call() {
        assert!(UNHANDLED_SHUTDOWN_ADVICE.contains("on_shutdown"));
        assert!(UNHANDLED_SHUTDOWN_ADVICE.contains("kill_timeout"));
    }

    #[test]
    fn two_malformed_lines_warn_once_and_the_loop_keeps_going() {
        let mut reader =
            Cursor::new(b"not json\nalso not json\n{\"kind\":\"shutdown\"}\n".to_vec());
        let outbox = Outbox::new(4);
        let dispatch = RwLock::new(Dispatch::default());
        let shutdown_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&shutdown_hits);
        dispatch
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register_shutdown(Box::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }));

        let warnings: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        let warn = |message: &str| warnings.lock().unwrap().push(message.to_string());

        reader_loop(&mut reader, &outbox, &dispatch, &warn);

        let collected = warnings.into_inner().unwrap();
        assert_eq!(
            collected.len(),
            1,
            "expected exactly one warning for two malformed lines, got {collected:?}"
        );
        assert_eq!(
            shutdown_hits.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the well-formed shutdown after the two bad lines was never reached"
        );
    }

    /// fails if end of stream leaves the outbox open, parking the writer
    /// thread in `take_batch` forever.
    #[test]
    fn end_of_stream_breaks_the_loop_and_closes_the_outbox() {
        let mut reader = Cursor::new(Vec::new());
        let outbox = Outbox::new(4);
        let dispatch = RwLock::new(Dispatch::default());
        let warn = |_: &str| {};

        reader_loop(&mut reader, &outbox, &dispatch, &warn);

        // Non-blocking probe: `push_blocking` on a closed outbox returns
        // `Err(Closed)` at once rather than waiting.
        // A dropped `close()` in `reader_loop` fails here in microseconds.
        // Otherwise this test hangs on an open, unbounded queue.
        assert!(
            outbox.push_blocking(ChildMessage::Ready).is_err(),
            "reader_loop returned without closing the outbox"
        );
        assert_eq!(
            outbox.take(),
            vec![],
            "outbox should read as closed-and-empty after EOF, not park a waiter"
        );
    }

    /// fails if a reply never reaches the outbox, or drops its id. The
    /// daemon needs that id to match a reply to its trigger.
    #[test]
    fn an_actions_reply_reaches_the_outbox_carrying_its_id() {
        let mut reader = Cursor::new(b"{\"kind\":\"action\",\"name\":\"gc\",\"id\":7}\n".to_vec());
        let outbox = Outbox::new(4);
        let dispatch = RwLock::new(Dispatch::default());
        dispatch
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register_action("gc".to_string(), Box::new(|_, _| "ok".to_string()));
        let warn = |_: &str| {};

        reader_loop(&mut reader, &outbox, &dispatch, &warn);

        match outbox.take().as_slice() {
            [ChildMessage::ActionReply { action, body, id }] => {
                assert_eq!(action, "gc");
                assert_eq!(body, "ok");
                assert_eq!(*id, Some(7));
            }
            other => panic!("expected an action reply, got {other:?}"),
        }
    }

    /// A shutdown reply queued right before the shepherd leaves must still
    /// reach the wire.
    #[test]
    fn the_writer_drains_what_is_already_queued_after_close() {
        let outbox = Outbox::new(4);
        outbox
            .push_blocking(ChildMessage::Ready)
            .expect("room for the first message");
        outbox
            .push_blocking(ChildMessage::Metric {
                name: "rps".to_string(),
                value: 1.0,
                metric_type: None,
                labels: std::collections::BTreeMap::new(),
                help: None,
            })
            .expect("room for the second message");
        outbox.close();

        let mut written = Vec::new();
        writer_loop(&mut written, &outbox);

        let text = String::from_utf8(written).expect("valid utf8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "expected both already-queued messages to be written, got {lines:?}"
        );
        assert!(lines[0].contains("\"kind\":\"ready\""));
        assert!(lines[1].contains("\"kind\":\"metric\""));
    }

    /// A burst already queued costs the transport one write, however many
    /// messages it holds, and every one of them still reaches the wire.
    #[test]
    fn the_writer_writes_a_queued_burst_in_one_write() {
        struct CountingSink {
            wire: Vec<u8>,
            writes: usize,
        }

        impl Write for CountingSink {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                self.wire.extend_from_slice(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let outbox = Outbox::new(8);
        for value in [1.0, 2.0, 3.0] {
            outbox.push_lossy(ChildMessage::Metric {
                name: "rps".to_string(),
                value,
                metric_type: None,
                labels: std::collections::BTreeMap::new(),
                help: None,
            });
        }
        outbox.close();

        let mut sink = CountingSink {
            wire: Vec::new(),
            writes: 0,
        };
        writer_loop(&mut sink, &outbox);

        assert_eq!(sink.writes, 1, "the burst was written message by message");
        assert_eq!(
            String::from_utf8(sink.wire)
                .expect("valid utf8")
                .lines()
                .count(),
            3
        );
        assert_eq!(outbox.pending(), 0, "a written burst is still pending");
    }
}
