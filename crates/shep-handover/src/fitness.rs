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
/// path. The gate does not need to check that the target reads the state's
/// format: [`Handover::exec`](crate::Handover::exec) asks the target itself
/// once the gate has said yes.
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
