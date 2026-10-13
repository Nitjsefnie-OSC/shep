//! The outgoing side: describing a handover and doing it.

use core::convert::Infallible;
use core::fmt;
use std::ffi::OsString;
use std::marker::PhantomData;
use std::path::PathBuf;

use crate::error::HandOverError;
use crate::fitness::Fitness;
use crate::state::State;

/// A handover being described: the state to pass, the descriptors to carry,
/// and the binary and arguments to exec.
///
/// Built with [`new`](Self::new), refined, and consumed by
/// [`exec`](Self::exec), which returns only if the handover did not happen.
///
/// `Debug` prints the format, the keys and the target, and leaves the state
/// out: it may carry a secret.
pub struct Handover<'a, S: State> {
    state: &'a S,
    #[cfg(unix)]
    fds: Vec<(String, std::os::fd::RawFd)>,
    target: Option<PathBuf>,
    arg0: Option<OsString>,
    args: Option<Vec<OsString>>,
    dir: Option<PathBuf>,
    /// Ties the borrowed descriptors to `'a` on a platform that stores none.
    borrow: PhantomData<&'a ()>,
}

impl<S: State> fmt::Debug for Handover<'_, S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut shown = f.debug_struct("Handover");
        shown.field("format", &S::FORMAT).field("state", &"..");
        #[cfg(unix)]
        shown.field(
            "carry",
            &self.fds.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        );
        shown
            .field("target", &self.target)
            .field("args", &self.args.as_ref().map(Vec::len))
            .finish_non_exhaustive()
    }
}

impl<'a, S: State> Handover<'a, S> {
    /// Describe a handover that passes `state`.
    ///
    /// By default it execs [`exec_target`](crate::exec_target) with this
    /// process's own arguments, and writes the blob into
    /// [`std::env::temp_dir`].
    #[must_use]
    pub const fn new(state: &'a S) -> Self {
        Self {
            state,
            #[cfg(unix)]
            fds: Vec::new(),
            target: None,
            arg0: None,
            args: None,
            dir: None,
            borrow: PhantomData,
        }
    }

    /// Carry `fd` across the exec under `key`; the new image claims it with
    /// [`Carried::take`](crate::Carried::take).
    ///
    /// Only the descriptors named here have close-on-exec cleared. Every
    /// other descriptor this process opened is closed in the new image, as
    /// long as it is close-on-exec, which is what Rust opens everything as.
    /// One inherited without the bit is the caller's to set.
    ///
    /// An order for the keys is not kept, and a key carried twice makes
    /// [`exec`](Self::exec) refuse.
    #[cfg(unix)]
    #[must_use]
    pub fn carry<F: std::os::fd::AsFd + ?Sized>(
        mut self,
        key: impl Into<String>,
        fd: &'a F,
    ) -> Self {
        use std::os::fd::AsRawFd as _;
        self.fds.push((key.into(), fd.as_fd().as_raw_fd()));
        self
    }

    /// Exec this binary instead of [`exec_target`](crate::exec_target).
    #[must_use]
    pub fn target(mut self, path: impl Into<PathBuf>) -> Self {
        self.target = Some(path.into());
        self
    }

    /// Pass these as `argv[1..]` instead of this process's own.
    #[must_use]
    pub fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<OsString>,
    {
        self.args = Some(args.into_iter().map(Into::into).collect());
        self
    }

    /// Pass this as `argv[0]` instead of this process's own, for a binary
    /// that picks its role from the name it was started under.
    #[must_use]
    pub fn arg0(mut self, arg0: impl Into<OsString>) -> Self {
        self.arg0 = Some(arg0.into());
        self
    }

    /// Write the blob into this directory instead of [`std::env::temp_dir`].
    ///
    /// The file is created at mode `0600` and removed by the new image once
    /// it has read it.
    #[must_use]
    pub fn blob_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dir = Some(dir.into());
        self
    }

    /// Replace this process's image with the target, carrying the state and
    /// the named descriptors.
    ///
    /// In order: resolve the target, ask `gate`, check the keys, encode the
    /// state, write the blob, clear close-on-exec on the carried descriptors
    /// and only those, and `execve`. The children this image started stay its
    /// children across the exec.
    ///
    /// # Errors
    ///
    /// Returns only if the handover did not happen, and then this image is
    /// still running with every descriptor close-on-exec again and no blob on
    /// disk; see [`HandOverError`] for each way. On a platform with no
    /// `execve` that is [`HandOverError::Unsupported`], always.
    pub fn exec(self, gate: &impl Fitness) -> Result<Infallible, HandOverError> {
        #[cfg(unix)]
        {
            self.exec_unix(gate)
        }
        #[cfg(not(unix))]
        {
            let _ = (
                gate,
                self.state,
                &self.target,
                &self.arg0,
                &self.args,
                &self.dir,
            );
            Err(HandOverError::Unsupported)
        }
    }

    #[cfg(unix)]
    fn exec_unix(self, gate: &impl Fitness) -> Result<Infallible, HandOverError> {
        use crate::fitness::Verdict;

        let target = match &self.target {
            Some(path) => crate::target::vet(path)
                .map(|()| path.clone())
                .map_err(HandOverError::Target)?,
            None => crate::exec_target().map_err(HandOverError::Target)?,
        };
        if let Verdict::Refused(reason) = gate.check(&target) {
            return Err(HandOverError::Refused { reason });
        }

        let mut numbers = std::collections::BTreeMap::new();
        for (key, fd) in &self.fds {
            if numbers.insert(key.clone(), *fd).is_some() {
                return Err(HandOverError::DuplicateKey { key: key.clone() });
            }
        }
        let blob = crate::envelope::encode(&numbers, self.state).map_err(HandOverError::State)?;

        let mut own = std::env::args_os();
        let own_arg0 = own.next();
        let arg0 = self
            .arg0
            .or(own_arg0)
            .unwrap_or_else(|| target.clone().into_os_string());
        let rest = self.args.unwrap_or_else(|| own.collect());
        let argv = std::iter::once(arg0).chain(rest).collect();

        let plan = crate::exec::Plan {
            target,
            argv,
            fds: self.fds,
            blob,
            dir: self.dir.unwrap_or_else(std::env::temp_dir),
        };
        crate::exec::run(&plan)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::Verdict;

    #[derive(Serialize, Deserialize)]
    struct Secret {
        token: String,
    }

    impl State for Secret {
        const FORMAT: u32 = 3;
    }

    #[cfg(unix)]
    #[test]
    fn debug_names_the_keys_and_never_the_state() {
        let state = Secret {
            token: "hunter2".to_owned(),
        };
        let (read, _write) = std::io::pipe().unwrap();
        let handover = Handover::new(&state)
            .carry("child-out", &read)
            .target("/opt/app/app")
            .args(["--serve"]);
        assert_eq!(
            format!("{handover:?}"),
            r#"Handover { format: 3, state: "..", carry: ["child-out"], target: Some("/opt/app/app"), args: Some(1), .. }"#
        );
    }

    #[test]
    fn a_gate_that_refuses_returns_its_reason_before_anything_is_touched() {
        let state = Secret {
            token: "t".to_owned(),
        };
        let dir = tempfile::tempdir().unwrap();
        let refusal = Handover::new(&state)
            .blob_dir(dir.path())
            .exec(&|_: &Path| Verdict::Refused("a pump is wedged".to_owned()))
            .unwrap_err();
        assert!(
            matches!(&refusal, HandOverError::Refused { reason } if reason == "a pump is wedged"),
            "{refusal}"
        );
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "no blob was written"
        );
    }

    #[test]
    fn the_gate_is_shown_the_resolved_target() {
        let state = Secret {
            token: "t".to_owned(),
        };
        let seen = std::cell::RefCell::new(None);
        let me = std::env::current_exe().unwrap();
        let _ = Handover::new(&state).target(&me).exec(&|target: &Path| {
            *seen.borrow_mut() = Some(target.to_path_buf());
            Verdict::Refused("looked".to_owned())
        });
        assert_eq!(seen.into_inner(), Some(me));
    }

    #[test]
    fn a_target_that_is_not_a_file_is_refused_before_the_gate_runs() {
        let state = Secret {
            token: "t".to_owned(),
        };
        let dir = tempfile::tempdir().unwrap();
        let err = Handover::new(&state)
            .target(dir.path().join("missing"))
            .exec(&|_: &Path| -> Verdict { panic!("the gate should not run") })
            .unwrap_err();
        assert!(matches!(err, HandOverError::Target(_)), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn one_key_carried_twice_is_refused() {
        let state = Secret {
            token: "t".to_owned(),
        };
        let (read, write) = std::io::pipe().unwrap();
        let err = Handover::new(&state)
            .carry("pipe", &read)
            .carry("pipe", &write)
            .exec(&|_: &Path| Verdict::Carryable)
            .unwrap_err();
        assert!(
            matches!(&err, HandOverError::DuplicateKey { key } if key == "pipe"),
            "{err}"
        );
    }
}
