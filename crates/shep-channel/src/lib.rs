//! Speak the shep shepherd channel: signal readiness, emit a metric,
//! answer an action. `docs/shepherd-channel.md` has the contract.
//!
//! With no channel, nothing is sent and no handler runs. Registering one
//! still succeeds, so an app needs no branch at its call sites.
//!
//! ```
//! let shepherd = shep_channel::serve();
//!
//! shepherd.on_action("gc", |params, _name| {
//!     format!("collected, params={params:?}")
//! });
//! shepherd.on_shutdown(|| { /* stop gracefully */ });
//!
//! // No channel here: this test runs outside shep. See `is_active`.
//! assert!(!shepherd.is_active());
//! shepherd.metric("rps", 4200.0);
//! shepherd.ready().unwrap();
//!
//! // Before the app exits, so a queued reply is not lost with it.
//! shepherd.flush(std::time::Duration::from_secs(2)).unwrap();
//! ```
//!
//! - An action's handler runs on a thread of its own, so a slow one does
//!   not delay the messages behind it. Up to 32 run at once, and past that
//!   the reader answers actions itself. Every reply carries the id of the
//!   action it answers, so a reply lands on the right trigger whatever
//!   order the handlers finish in. `action_timeout` (default 3s) is still
//!   the budget one handler gets.
//! - `metric` and `note` drop under backpressure. `ready` and a reply block
//!   for room instead, since losing either is worse.
//! - Sending queues; a writer thread does the writing. An app that ends
//!   itself, typically from `on_shutdown`, calls `flush` first or loses
//!   whatever had not been written yet.
//! - An unhandled shutdown warns on stderr. This crate never stops a
//!   process on its own.

#![doc(test(attr(deny(warnings))))]
// Not `forbid`: `endpoint` needs unsafe to reach the channel. Three blocks
// on unix probe an inherited descriptor and then take it: the channel's
// own, and a dog's connection to its shepherd, which `shep-client` cannot
// adopt itself. One on Windows peeks a pipe's buffer. Each carries its own
// `// SAFETY:` comment.
#![deny(unsafe_code)]

#[cfg(feature = "client")]
mod channel;
#[cfg(feature = "client")]
mod dispatch;
#[cfg(feature = "client")]
mod endpoint;
#[cfg(feature = "client")]
mod error;
#[cfg(feature = "client")]
mod loops;
#[cfg(feature = "client")]
mod metric;
#[cfg(feature = "client")]
mod outbox;
#[cfg(feature = "client")]
mod serve;
#[cfg(feature = "client")]
mod session;
mod wire;

#[cfg(feature = "client")]
pub use channel::Channel;
#[cfg(feature = "client")]
pub use dispatch::{ActionHandler, ShutdownHandler};
#[cfg(feature = "client")]
pub use endpoint::{Endpoint, FD_VAR, PIPE_VAR, VERSION_VAR, discover};
#[cfg(all(feature = "client", unix))]
pub use endpoint::take_inherited_socket;
#[cfg(feature = "client")]
pub use error::ChannelError;
#[cfg(feature = "client")]
pub use metric::MetricOptions;
#[cfg(feature = "client")]
pub use serve::{Shepherd, serve};
pub use wire::{
    CHANNEL_VERSION, ChildMessage, LambLabel, LambLabelError, MetricType, ShepherdMessage,
};
