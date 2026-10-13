//! The disabled mark: which apps an operator parked, and the refusals that
//! keep a spawn from undoing it.

use super::*;

/// Which side of the disabled mark a wildcard reaches.
///
/// An exact selector ignores it and reaches its target either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sweep {
    /// Sheep that are not disabled: every verb but `enable`.
    Enabled,
    /// Sheep that are disabled: `enable`.
    Disabled,
    /// Every sheep, parked or not: the log verbs, which read or empty files a
    /// disabled app still owns.
    Any,
}

impl Sweep {
    /// Whether a wildcard on this side reaches a sheep whose app is
    /// `disabled`.
    pub(super) fn reaches(self, disabled: bool) -> bool {
        match self {
            Self::Enabled => !disabled,
            Self::Disabled => disabled,
            Self::Any => true,
        }
    }
}

impl<R: ProcessRunner> Actor<R> {
    /// Disables the app of every sheep `selector` reaches and stops it.
    ///
    /// Widened to the whole app: the mark is keyed by name, so a selector
    /// that matched one instance disables all of them. The mark is set before
    /// the stop begins, so nothing the stop wakes can respawn under it.
    pub(super) fn handle_disable(
        &mut self,
        selector: &ProcessSelector,
        reply: oneshot::Sender<Result<Vec<ProcessInfo>, SupervisorError>>,
    ) {
        let names = match self.apps_reached(selector, Sweep::Enabled) {
            Ok(names) => names,
            // A sweep over sheep all of which are parked has nothing to
            // disable, which is the answer an exact name gets too.
            Err(SupervisorError::NotFound)
                if !self.reached_ids(selector, Sweep::Disabled).is_empty() =>
            {
                let _ = reply.send(Ok(Vec::new()));
                return;
            }
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        let ids = self.ids_of_apps(&names);
        // The mark wins over what is in flight: a restart's kill marker
        // becomes a stop, and a serial reload mid-drain is abandoned so the
        // drained exit leaves a stopped row rather than a replacement.
        for slot in self.sheep.values_mut() {
            let restarting = slot.manual.is_some_and(|m| m.kind == ManualKind::Restart);
            if restarting && names.contains(&slot.entry.spec.config().name) {
                slot.manual = Some(PendingManual {
                    kind: ManualKind::Stop,
                    origin: CommandOrigin::Operator,
                });
            }
        }
        for name in &names {
            self.abandon_serial_drain(name);
        }
        self.disabled.extend(names);
        self.begin_manual_ids(
            ids,
            ManualKind::Stop,
            CommandOrigin::Operator,
            ReplyKind::Info(reply),
        );
    }

    /// Ends `name`'s reload while it is draining an instance before any
    /// replacement exists, and takes the drain marker off that instance so
    /// its exit is an ordinary stop.
    ///
    /// Any other phase is left alone. A serial reload's replacement is
    /// already registered and phased `DrainOld` by then, so its exit under the
    /// disable's stop goes the ordinary route and leaves a `Stopped` row; an
    /// overlap swap keeps the old instance's row the same way.
    fn abandon_serial_drain(&mut self, name: &str) {
        let Some(job) = self.reloads.get(name) else {
            return;
        };
        if job.swap.phase != ReloadPhase::DrainFirst {
            return;
        }
        let old_id = job.swap.old_id;
        tracing::warn!(
            name,
            old_id,
            "reload abandoned: the app was disabled, so no replacement is spawned"
        );
        self.reloads.remove(name);
        self.clear_reload(old_id);
        if let Some(slot) = self.sheep.get(&old_id) {
            let info = to_info(&slot.entry, &self.smits);
            self.emit(ProcessEventKind::ReloadAbandoned, info, true);
        }
    }

    /// Marks the apps of `apps` that `names` lists disabled, ahead of their
    /// registration at rest.
    ///
    /// A name outside `apps` is dropped: with no registered instance the
    /// actor would prune it as soon as this message ends.
    pub(super) fn mark_disabled(&mut self, apps: &[ResolvedApp], names: BTreeSet<String>) {
        self.disabled.extend(
            apps.iter()
                .map(|app| &app.config().name)
                .filter(|name| names.contains(*name))
                .cloned(),
        );
    }

    /// Clears the mark on every disabled app `selector` reaches and starts
    /// it. An app that was not disabled is left as it is.
    pub(super) fn handle_enable(
        &mut self,
        selector: &ProcessSelector,
        reply: oneshot::Sender<Result<Vec<ProcessInfo>, SupervisorError>>,
    ) {
        let names = match self.apps_reached(selector, Sweep::Disabled) {
            Ok(names) => names,
            // A sweep over sheep none of which is parked has nothing to
            // enable, which is the answer an exact name gets too.
            Err(SupervisorError::NotFound) if !self.matching_ids(selector).is_empty() => {
                let _ = reply.send(Ok(Vec::new()));
                return;
            }
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        if self.shutting_down {
            let _ = reply.send(Err(SupervisorError::EngineStopped));
            return;
        }
        let cleared: BTreeSet<String> = names
            .into_iter()
            .filter(|name| self.disabled.remove(name))
            .collect();
        let ids = self.ids_of_apps(&cleared);
        // A stop a disable already started owns the next exit, and the
        // restart below would ride it. The enable is the later word.
        for slot in self.sheep.values_mut() {
            let stopping = slot.manual.is_some_and(|m| m.kind == ManualKind::Stop);
            if stopping && cleared.contains(&slot.entry.spec.config().name) {
                slot.manual = Some(PendingManual {
                    kind: ManualKind::Restart,
                    origin: CommandOrigin::Operator,
                });
            }
        }
        self.begin_manual_ids(
            ids,
            ManualKind::Restart,
            CommandOrigin::Operator,
            ReplyKind::Info(reply),
        );
    }

    /// Restarts what `selector` reaches, leaving a disabled app stopped.
    ///
    /// A selector that names the app (an id, a name, a slot) is refused with
    /// [`SupervisorError::Disabled`] when `origin` is an operator's. A sweep,
    /// and anything the daemon raised itself, passes the app by.
    pub(super) fn restart_unless_disabled(
        &mut self,
        selector: &ProcessSelector,
        origin: CommandOrigin,
        reply: ReplyKind,
    ) {
        let matched = self.matching_ids(selector);
        let (held, ids): (Vec<u32>, Vec<u32>) =
            matched.iter().partition(|id| self.is_disabled_id(**id));
        if matched.is_empty() {
            send_reply(reply, Err(SupervisorError::NotFound));
            return;
        }
        if let Some(name) = held.first().and_then(|id| self.name_of(*id)) {
            if origin == CommandOrigin::Operator && selector.is_exact() {
                send_reply(reply, Err(SupervisorError::Disabled(name)));
                return;
            }
            tracing::debug!(sheep = %name, "restart dropped: the app is disabled");
        }
        self.begin_manual_ids(ids, ManualKind::Restart, origin, reply);
    }

    /// The first app of `names` that is disabled, as a refusal.
    ///
    /// # Errors
    ///
    /// [`SupervisorError::Disabled`] naming that app.
    pub(super) fn refuse_disabled<'a>(
        &self,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), SupervisorError> {
        match names.into_iter().find(|name| self.disabled.contains(*name)) {
            Some(name) => Err(SupervisorError::Disabled(name.to_string())),
            None => Ok(()),
        }
    }

    /// Stamps the mark onto a listing row, for the rows `to_info` builds
    /// without the set.
    pub(super) fn with_mark(&self, mut info: ProcessInfo) -> ProcessInfo {
        if self.disabled.contains(&info.name) {
            info.disabled = Some(true);
        }
        info
    }

    /// Forgets the mark of an app that has no registered instance left, so a
    /// deleted app's name is free again.
    pub(super) fn prune_disabled(&mut self) {
        if self.disabled.is_empty() {
            return;
        }
        let held: BTreeSet<&str> = self
            .sheep
            .values()
            .map(|slot| slot.entry.spec.config().name.as_str())
            .collect();
        self.disabled.retain(|name| held.contains(name.as_str()));
    }

    /// The distinct apps `selector` reaches, dogs excluded.
    fn apps_reached(
        &self,
        selector: &ProcessSelector,
        sweep: Sweep,
    ) -> Result<BTreeSet<String>, SupervisorError> {
        let matched = self.reached_ids(selector, sweep);
        if matched.is_empty() {
            return Err(SupervisorError::NotFound);
        }
        let names: BTreeSet<String> = matched
            .iter()
            .filter_map(|id| self.sheep.get(id))
            .filter(|slot| slot.entry.dog.is_none())
            .map(|slot| slot.entry.spec.config().name.clone())
            .collect();
        if names.is_empty() {
            return Err(SupervisorError::IsADog(
                "a dog is enabled and disabled by name, and `shep disable` and `shep enable` \
                 reach it that way"
                    .to_string(),
            ));
        }
        Ok(names)
    }

    /// Every registered id of the apps in `names`, in id order.
    fn ids_of_apps(&self, names: &BTreeSet<String>) -> Vec<u32> {
        let mut ids: Vec<u32> = self
            .sheep
            .iter()
            .filter(|(_, slot)| names.contains(&slot.entry.spec.config().name))
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn name_of(&self, id: u32) -> Option<String> {
        self.sheep
            .get(&id)
            .map(|slot| slot.entry.spec.config().name.clone())
    }

    pub(super) fn is_disabled_id(&self, id: u32) -> bool {
        self.sheep
            .get(&id)
            .is_some_and(|slot| self.disabled.contains(&slot.entry.spec.config().name))
    }
}
