//! A frame decoder that knows whether it holds part of a frame.
//!
//! A handover can only carry a connection whose next byte starts a frame. A
//! `FramedRead`'s buffer is not enough to say so: the length codec consumes a
//! frame's 4-byte header as soon as it arrives, so a frame still waiting for
//! its payload can leave the buffer empty.

use bytes::BytesMut;
use std::io;
use tokio_util::codec::{Decoder, LengthDelimitedCodec};

/// [`LengthDelimitedCodec`] that remembers whether its last decode left bytes
/// of a frame taken off the socket and not yet handed on.
#[derive(Debug)]
pub(super) struct TrackedCodec {
    inner: LengthDelimitedCodec,
    holding: bool,
}

impl TrackedCodec {
    /// Wraps `inner`, holding nothing.
    pub(super) const fn new(inner: LengthDelimitedCodec) -> Self {
        Self {
            inner,
            holding: false,
        }
    }

    /// Whether bytes already read from the socket belong to a frame that has
    /// not been delivered, so that the socket itself no longer starts one.
    pub(super) const fn holding(&self) -> bool {
        self.holding
    }
}

impl Decoder for TrackedCodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<BytesMut>> {
        let before = src.len();
        let item = self.inner.decode(src)?;
        // A frame handed on with bytes left over is the start of the next one.
        // No frame handed on with the buffer shorter than it was is a header
        // taken and a payload awaited.
        self.holding = match item {
            Some(_) => !src.is_empty(),
            None => !src.is_empty() || src.len() < before,
        };
        Ok(item)
    }
}

#[cfg(test)]
mod tests;
