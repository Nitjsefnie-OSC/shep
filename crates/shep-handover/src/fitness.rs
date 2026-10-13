//! The fitness gate: the caller's say over whether a handover goes ahead.

use std::path::Path;

/// The gate's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Go ahead.
    Carryable,
    /// Do not hand over at all, and why.
    ///
    /// The reason reaches the caller in
    /// [`HandOverError::Refused`](crate::HandOverError::Refused), so it should
    /// say what happens instead.
    Refused(String),
}

/// A caller's gate over a handover, asked once the target is resolved and
/// before anything is written, cleared or exec'd.
///
/// It refuses whole: one refusal sends the caller down its stop-and-start
/// path. It is also the old image's one chance to look at the target, for
/// instance to run it with a flag and read which state formats it reads,
/// since a build that cannot read the state can only refuse it after the old
/// image is gone.
///
/// Any `Fn(&Path) -> Verdict` is a gate.
pub trait Fitness {
    /// Whether this process can be replaced by `target` right now.
    fn check(&self, target: &Path) -> Verdict;
}

impl<F: Fn(&Path) -> Verdict> Fitness for F {
    fn check(&self, target: &Path) -> Verdict {
        self(target)
    }
}
