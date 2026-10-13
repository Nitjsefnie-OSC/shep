//! The pane itself: what it is over, and what it reads off that target.
//!
//! One `FieldSet`, the values the target currently holds, and a `Viewport`
//! over the rows. Everything here answers a question; the answers to
//! keystrokes live in the modules beside it, and the rows themselves and the
//! cursor's walk through them in [`super::walk`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Map, Value};
use shep_core::config::{ApplyGroup, apply_group, reaches_running};
use shep_core::protocol::SheepConfigView;

use super::super::edits::{EditKey, Edits};
use super::super::field::{
    Field, FieldKind, FieldSet, Flattened, Outline, flatten_values, flattened,
};
use super::super::viewport::Viewport;
use super::entries::{EntryBody, EntryRows};
use super::fields::{entries_text, render_json, resolved_by_kind, resolved_display, sheep_fields};
use super::sheep_dog::SheepDogTable;

// Link-only (IR-32): the unit grammars a row's resolved display goes
// through, the field kind a locked row reports, and the group order this
// type's own field indexes into.
#[cfg(doc)]
use super::super::field::ValueKind;
use super::dogs::dog_names;
use super::{DogsPane, EnvTyping, ListPane, Lock, PaneEdit, PaneRow, PaneTarget, PaneTyping};
#[cfg(doc)]
use shep_core::config::GROUP_ORDER;
#[cfg(doc)]
use shep_core::values::{MemSize, UpDuration};

/// The state of an open pane.
///
/// `Debug` is manual and redacted (IR-41): `values` is a sheep's config with
/// `env` already stripped by [`SheepConfigView::new`], but `args` and `cwd`
/// are still in it and routinely carry a token or a home directory.
/// `env_keys` is a key set, which is itself worth keeping out of a log, and
/// the same reasoning [`SheepConfigView`]'s own `Debug` gives applies here
/// unchanged: this type is a copy of that one's payload.
#[derive(Clone)]
pub struct ConfigPane {
    pub(super) target: PaneTarget,
    pub(super) fields: FieldSet,
    pub(super) values: Map<String, Value>,
    pub(super) env_keys: Vec<String>,
    pub(super) overridden: Vec<String>,
    /// Field names parked until the next respawn, as the shepherd reported
    /// them. Nothing to do with [`Self::edits`], which is this pane's own
    /// set of unwritten changes; the two words come from opposite ends and
    /// the collision is the shepherd's.
    pub(super) pending: Vec<String>,
    /// Index into [`GROUP_ORDER`]: which group's fields the field list
    /// draws. Meaningless for a dog pane, whose fields carry no group and
    /// so are visible under any of them; see [`Self::rows`].
    pub(super) group: usize,
    pub(super) view: Viewport,
    /// The open text editor, or [`None`].
    pub(super) typing: Option<PaneTyping>,
    /// Everything the operator has changed and nothing has written yet.
    /// Emptied by [`Self::close`], which is the only door out.
    pub(super) edits: Edits,
    /// The open env editor, or [`None`]. Never open at the same time as
    /// [`Self::list`]: each opens on a row of its own kind, and `Escape`
    /// closes whichever is up before the pane.
    pub(super) env_typing: Option<EnvTyping>,
    /// The open list sub-screen. Never open at the same time as
    /// [`Self::env_typing`]: each opens on a row of its own kind, and
    /// `Escape` closes whichever is up before the pane. `Box`, like
    /// [`Self::dogs`]: one is open at a time, and inline it would put a
    /// whole sub-screen in every move of the pane.
    pub(super) list: Option<Box<ListPane>>,
    /// The open dogs sub-screen, over a sheep's `dogs` row. Never open at
    /// the same time as [`Self::list`] or an editor: it opens only on a
    /// field list with nothing else up.
    pub(super) dogs: Option<Box<DogsPane>>,
    /// The dog's `[<name>]` table as TOML text, and [`None`] for a sheep.
    ///
    /// Kept beside the parsed `values` rather than instead of them, because
    /// the two answer different questions: `values` is what the rows render,
    /// and this is what a write edits. `Request::SetDogConfig` replaces the
    /// whole section, so an edit that re-rendered it from `values` would
    /// throw away every comment the operator wrote. See
    /// [`Self::edited_section_with`].
    pub(super) section: Option<String>,
    /// A dog's table on a sheep, whole, and where each dotted row lives in
    /// it. [`None`] for every other target. See [`Self::sheep_dog`].
    pub(super) dog_table: Option<Box<SheepDogTable>>,
    /// Each dotted row key of a dog pane to the real property path it was
    /// built from. Empty for any other target.
    ///
    /// The rows are [`flattened`]'s, so a key is dotted for a nested table
    /// and quoted for a property literally named with a dot. The write walks
    /// this map rather than the key: see [`Self::edited_section_with`].
    pub(super) section_paths: BTreeMap<String, Vec<String>>,
    /// The tables a dog pane's rows nest in. Empty for a sheep. Shared, so
    /// the pane stays small enough to sit in `Body` unboxed.
    pub(super) outline: Arc<Outline>,
    /// The rows under each array or map of tables. Empty for a sheep.
    pub(super) entries: Arc<EntryRows>,
    /// The tables currently drawn open, by their dotted path. See
    /// [`Self::rows`].
    pub(super) open: BTreeSet<String>,
}

impl core::fmt::Debug for ConfigPane {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ConfigPane {{ target: {:?}, fields: {}, env_keys: {}, cursor: {} }}",
            self.target,
            self.fields.len(),
            self.env_keys.len(),
            self.view.cursor()
        )
    }
}

impl ConfigPane {
    /// A pane over one sheep's config, read off the Flockfile schema.
    ///
    /// The schema is the field list: it already carries every property's
    /// type, default and group, so the pane reads the same document `shep
    /// init` scaffolds from rather than keeping a second list of 40 names
    /// in step with it.
    #[must_use]
    pub fn sheep(view: SheepConfigView) -> Self {
        let (fields, values) = sheep_fields(&view.config);
        Self {
            target: PaneTarget::Sheep { name: view.name },
            fields,
            values,
            env_keys: view.env_keys,
            overridden: view.overridden,
            pending: view.pending,
            group: 0,
            view: Viewport::new(),
            typing: None,
            edits: Edits::default(),
            env_typing: None,
            list: None,
            dogs: None,
            section: None,
            dog_table: None,
            section_paths: BTreeMap::new(),
            outline: Arc::new(Outline::default()),
            entries: Arc::default(),
            open: BTreeSet::new(),
        }
    }

    /// A pane over one dog's `[<name>]` section.
    ///
    /// `schema` is the dog's own answer to the schema flag, probed at
    /// open rather than read from anywhere; `section` is the table as
    /// `Request::DogConfig` rendered it, empty when `dogs.toml` has none.
    ///
    /// In schema order, with no group headers: a dog's schema carries no
    /// `init.group`. A nested table is a row that opens and closes, with its
    /// keys under it, as it is on that dog's table over a sheep; see
    /// [`Self::rows`]. A [`FieldKind::List`] row opens the list sub-screen,
    /// as a sheep's `args` does. A [`FieldKind::Map`] row, and a list
    /// [`flattened`] closed to [`FieldKind::Opaque`], is marked not editable
    /// and draws [`Lock::NoWidget`].
    #[must_use]
    pub fn dog(
        name: String,
        adopted_path: Option<PathBuf>,
        schema: Value,
        section: String,
    ) -> Self {
        let Flattened {
            fields,
            paths,
            outline,
            shapes,
        } = flattened(&schema);
        let fields = FieldSet::from_fields(
            fields
                .fields()
                .iter()
                .cloned()
                .map(|mut field| {
                    if field.kind == FieldKind::Map {
                        field.editable = false;
                    }
                    field
                })
                .collect(),
            &[],
        );
        let table: Map<String, Value> = section
            .parse::<toml::Table>()
            .ok()
            .and_then(|table| serde_json::to_value(table).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let values = flatten_values(&table, &paths);
        let entries = EntryRows::build(&fields, &paths, &values, shapes);
        Self {
            target: PaneTarget::Dog { name, adopted_path },
            fields,
            values,
            env_keys: Vec::new(),
            overridden: Vec::new(),
            pending: Vec::new(),
            group: 0,
            view: Viewport::new(),
            typing: None,
            edits: Edits::default(),
            env_typing: None,
            list: None,
            dogs: None,
            section: Some(section),
            dog_table: None,
            section_paths: paths,
            open: outline.top_level_paths(),
            outline: Arc::new(outline),
            entries: Arc::new(entries),
        }
    }

    /// Whether `section` is the text this dog pane last took from the
    /// shepherd, comments included. False for any other target.
    #[must_use]
    pub(in crate::lookout) fn holds_section(&self, section: &str) -> bool {
        self.section.as_deref() == Some(section)
    }

    /// The key under the cursor, and why the pane will not edit it, when it
    /// will not. [`None`] both for a row that edits and for no row at all.
    ///
    /// The one place a caller asks "may I edit what is selected", so a
    /// refusal is raised for the right one of [`Lock`]'s two reasons rather
    /// than for a generic third.
    #[must_use]
    pub fn cursor_lock(&self) -> Option<(&str, Lock)> {
        let field = match self.cursor()? {
            PaneRow::Field(index) => self.fields.fields().get(index)?,
            PaneRow::Entry(index) => match &self.entries.row(index)?.body {
                EntryBody::Leaf { field, .. } => field,
                EntryBody::Node { .. } => return None,
            },
            PaneRow::Table(_) | PaneRow::Env(_) | PaneRow::AddEnv => return None,
        };
        self.lock(&field.key).map(|lock| (field.key.as_str(), lock))
    }

    /// The field under the cursor, for a row that holds a value: a config
    /// field, or a field inside an entry, whose key is its full path.
    pub(in crate::lookout) fn cursor_field(&self) -> Option<&Field> {
        match self.cursor()? {
            PaneRow::Field(index) => self.fields.fields().get(index),
            PaneRow::Entry(index) => match &self.entries.row(index)?.body {
                EntryBody::Leaf { field, .. } => Some(field),
                EntryBody::Node { .. } => None,
            },
            PaneRow::Table(_) | PaneRow::Env(_) | PaneRow::AddEnv => None,
        }
    }

    /// The field filed under `key`: one of [`Self::fields`], or a field
    /// inside an entry.
    pub(super) fn field_of(&self, key: &str) -> Option<&Field> {
        self.fields
            .by_key(key)
            .or_else(|| self.entries.leaf(key).map(|leaf| leaf.field))
    }

    /// What the target holds for `key`, or [`None`] for a key it holds
    /// nothing for. A field inside an entry reads [`Value::Null`] when the
    /// entry lacks it.
    pub(super) fn stored_of(&self, key: &str) -> Option<&Value> {
        self.values
            .get(key)
            .or_else(|| self.entries.leaf(key).map(|leaf| leaf.stored))
    }

    /// The kind of widget the row under the cursor wants, or [`None`] for
    /// no row at all, or for one on an env row, which has no [`FieldKind`].
    #[must_use]
    pub fn cursor_kind(&self) -> Option<&FieldKind> {
        match self.cursor()? {
            PaneRow::Field(index) => self.fields.fields().get(index).map(|field| &field.kind),
            PaneRow::Entry(index) => match &self.entries.row(index)?.body {
                EntryBody::Leaf { field, .. } => Some(&field.kind),
                EntryBody::Node { .. } => None,
            },
            PaneRow::Table(_) | PaneRow::Env(_) | PaneRow::AddEnv => None,
        }
    }

    /// What is being edited.
    #[must_use]
    pub fn target(&self) -> &PaneTarget {
        &self.target
    }

    /// The form.
    #[must_use]
    pub fn fields(&self) -> &FieldSet {
        &self.fields
    }

    /// The current value of `key`, rendered for a cell.
    ///
    /// A scalar shows bare, an absent or `null` value shows `(unset)`, and
    /// anything else shows compact JSON. On a dog's pane an array or map of
    /// tables shows how many entries it holds and nothing of them, since an
    /// entry can hold a secret. A sheep's `env` is the one field
    /// whose value this pane never holds, since the shepherd strips it on
    /// the way out, so it shows its key count instead, and the sub-screen
    /// shows the names. A sheep's `dogs` shows dog names and no value: a
    /// table's secrets are marked only by a schema this pane does not hold.
    ///
    /// That special case is gated on the target, not on the key alone: a
    /// dog's schema is somebody else's, and one declaring a field named
    /// `env` reads its own section, not [`Self::env_keys`], which
    /// [`Self::dog`] always leaves empty.
    ///
    /// This is the parseable form: what [`Self::begin_typing`] seeds an
    /// editor with. [`Self::display_value`] is the one a row draws, and the
    /// two disagree exactly for a [`ValueKind`] field, where a seed has to
    /// stay something the daemon's own grammar still accepts.
    #[must_use]
    pub fn value(&self, key: &str) -> String {
        if key == "env" && matches!(self.target, PaneTarget::Sheep { .. }) {
            return match self.env_keys.len() {
                1 => "1 key".to_owned(),
                count => format!("{count} keys"),
            };
        }
        if key == "dogs" && matches!(self.target, PaneTarget::Sheep { .. }) {
            return dog_names(self.values.get("dogs"));
        }
        if !matches!(self.target, PaneTarget::Sheep { .. })
            && let Some(FieldKind::Entries(_)) = self.fields.by_key(key).map(|field| &field.kind)
        {
            return entries_text(self.values.get(key));
        }
        if let Some(EntryBody::Leaf { text, .. }) = self.entries.leaf_body(key) {
            return text.clone();
        }
        self.values
            .get(key)
            .map_or_else(|| render_json(&Value::Null), render_json)
    }

    /// The value a filed edit holds for `key`, rendered the way
    /// [`Self::value`] renders a stored one, or [`None`] when nothing is
    /// filed for it. Only ever answers for a config field: an env edit has
    /// no field key to be filed under.
    #[must_use]
    pub fn edited_value(&self, key: &str) -> Option<String> {
        let PaneEdit::Set { value, .. } = self.edits.get(&EditKey::Field(key.to_owned()))?.edit()
        else {
            return None;
        };
        let raw = render_json(value.as_value());
        Some(self.resolved(key, &raw))
    }

    /// [`Self::value`], resolved through its own grammar for a
    /// [`MemSize`]/[`UpDuration`] field: what a row draws instead of the
    /// digits an operator would otherwise have to already know the
    /// convention for.
    #[must_use]
    pub fn display_value(&self, key: &str) -> String {
        self.resolved(key, &self.value(key))
    }

    fn resolved(&self, key: &str, raw: &str) -> String {
        match self.entries.leaf(key) {
            Some(leaf) => resolved_by_kind(leaf.field.value_kind, raw),
            None => resolved_display(&self.fields, key, raw),
        }
    }

    /// What changing `key` costs.
    ///
    /// [`None`] is not reachable for a sheep and is not dead weight either:
    /// a dog decides for itself what a published change reloads, so the
    /// answer for one is "the pane does not know", and every caller already
    /// renders that as an empty cost cell rather than as a guess.
    #[must_use]
    pub fn cost(&self, key: &str) -> Option<ApplyGroup> {
        match self.target {
            PaneTarget::Sheep { .. } => Some(apply_group(key)),
            // The dog decides, not shep. Said once at the foot of the pane
            // rather than guessed per row.
            PaneTarget::Dog { .. } | PaneTarget::SheepDog { .. } => None,
        }
    }

    /// Why the pane will not edit `key`, or [`None`] when it will.
    ///
    /// [`Lock::Refused`] outranks [`Lock::NoWidget`]: a Structural field
    /// that also happened to have no widget is still refused by shep, which
    /// is the fact that survives the pane gaining every widget it lacks. A
    /// sheep's `dogs` is [`Lock::SubScreen`] ahead of its own missing
    /// widget, since Enter there does open something.
    #[must_use]
    pub fn lock(&self, key: &str) -> Option<Lock> {
        if self.cost(key) == Some(ApplyGroup::Structural) {
            return Some(Lock::Refused);
        }
        if key == "dogs" && matches!(self.target, PaneTarget::Sheep { .. }) {
            return Some(Lock::SubScreen);
        }
        if let Some(leaf) = self.entries.leaf(key) {
            let edits_here = matches!(self.target, PaneTarget::Dog { .. })
                && matches!(
                    leaf.field.kind,
                    FieldKind::Bool
                        | FieldKind::Integer
                        | FieldKind::Text
                        | FieldKind::Choice(_)
                        | FieldKind::Suggested(_)
                );
            return (!edits_here || !leaf.field.editable).then_some(Lock::NoWidget);
        }
        match self.fields.by_key(key) {
            Some(field) if !field.editable => Some(Lock::NoWidget),
            _ => None,
        }
    }

    /// Whether an operator has overridden `key`.
    #[must_use]
    pub fn is_overridden(&self, key: &str) -> bool {
        self.overridden.iter().any(|name| name == key)
    }

    /// Whether `key` is parked until the next respawn.
    #[must_use]
    pub fn is_pending(&self, key: &str) -> bool {
        self.pending.iter().any(|name| name == key)
    }

    /// How many fields wait for a reload or a restart.
    ///
    /// Every one of them is already written to the override store, so this
    /// counts what the running process has not taken yet, never what an
    /// operator could lose.
    #[must_use]
    pub fn parked_count(&self) -> usize {
        self.pending.len()
    }

    /// The filed edits a respawn is what applies, by field name, in the
    /// set's own key order.
    ///
    /// An env key counts: `env` is `ApplyGroup::NeedsRespawn` and every
    /// value is baked into the child at exec.
    #[must_use]
    pub(in crate::lookout) fn unsent_fields_needing_a_respawn(&self) -> Vec<String> {
        self.edits
            .iter()
            .filter_map(|(key, _)| match key {
                EditKey::Field(name) if !reaches_running(name) => Some(name.clone()),
                EditKey::Env(name) => Some(format!("env {name}")),
                EditKey::Field(_) => None,
            })
            .collect()
    }

    /// How many filed edits the running sheep already takes without a
    /// respawn: the complement of [`Self::unsent_fields_needing_a_respawn`]
    /// within the same set. An env key never counts, for the same reason it
    /// always counts on the other side: `env` is `ApplyGroup::NeedsRespawn`.
    ///
    /// What the close dialog's "everything else you changed is already
    /// live" sentence draws on: a filed set holding only fields a respawn
    /// applies has nothing else to say that about.
    #[must_use]
    pub(in crate::lookout) fn live_edit_count(&self) -> usize {
        self.edits
            .iter()
            .filter(|(key, _)| matches!(key, EditKey::Field(name) if reaches_running(name)))
            .count()
    }

    /// Whether a reload of this sheep overlaps its replacement or runs
    /// serially. Always [`ReloadKind::Overlap`] for a dog, which has no
    /// such fields to read.
    #[must_use]
    pub fn reload_kind(&self) -> ReloadKind {
        let flag = |key: &str| {
            self.values
                .get(key)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let has_probe = self
            .values
            .get("readiness_probe")
            .is_some_and(|probe| !probe.is_null());
        reload_mode(flag("wait_ready"), has_probe, flag("reuse_port"))
    }

    /// Adopts a previous pane's pending set, so a refresh does not
    /// silently drop changes the operator has not written yet.
    ///
    /// The set is the operator's and the values are the shepherd's: a
    /// re-read replaces every value on screen and keeps every edit filed
    /// over them.
    ///
    /// An open editor is deliberately not carried: its buffer was seeded
    /// from a value this refresh may have just changed, so keeping it
    /// would put the operator halfway through editing something that is no
    /// longer there. `App::release_text_mode_if_unowned` is what puts the
    /// keyboard back when that happens.
    pub(in crate::lookout) fn adopt_edits(&mut self, previous: Edits) {
        self.edits = previous;
    }
}

/// Which of the daemon's two reloads an app takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReloadKind {
    /// The replacement is spawned alongside the instance it replaces.
    Overlap,
    /// The instance being replaced is drained first, so the app is down for
    /// the length of the drain.
    Serial,
}

/// Whether a reload of this app overlaps or runs serially.
///
/// The daemon decides through `ReadinessSource::of`, which answers for
/// `wait_ready` before it reads `readiness_probe`, so an app with both
/// overlaps.
const fn reload_mode(wait_ready: bool, has_probe: bool, reuse_port: bool) -> ReloadKind {
    if !wait_ready && has_probe && !reuse_port {
        ReloadKind::Serial
    } else {
        ReloadKind::Overlap
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::{bark_pane, web, web_with_args};
    use shep_core::values::{MemSize, UpDuration};

    use shep_core::config::{AppConfig, ProbeConfig, ProbeKind};

    use super::*;

    #[test]
    fn a_sheep_pane_has_forty_four_fields_in_nine_groups() {
        let pane = ConfigPane::sheep(web());
        assert_eq!(pane.fields().len(), 44);
        let mut groups: Vec<&str> = Vec::new();
        for field in pane.fields().fields() {
            let group = field.group.as_deref().expect("every field carries a group");
            if groups.last() != Some(&group) {
                groups.push(group);
            }
        }
        assert_eq!(
            groups,
            [
                "process",
                "logging",
                "inputs",
                "restart",
                "readiness",
                "shutdown",
                "watch",
                "cron",
                "dogs"
            ]
        );
    }

    #[test]
    fn a_value_renders_bare_for_a_scalar_and_as_a_count_for_env() {
        let pane = ConfigPane::sheep(web());
        assert_eq!(pane.value("max_restarts"), "32");
        assert_eq!(pane.value("autorestart"), "true");
        assert_eq!(pane.value("cwd"), "(unset)");
        assert_eq!(pane.value("env"), "1 key");
    }

    #[test]
    fn cost_comes_from_apply_group_for_a_sheep() {
        let pane = ConfigPane::sheep(web());
        assert_eq!(pane.cost("max_restarts"), Some(ApplyGroup::Live));
        assert_eq!(pane.cost("kill_signal"), Some(ApplyGroup::NextSpawn));
        assert_eq!(pane.cost("script"), Some(ApplyGroup::NeedsRespawn));
        assert_eq!(pane.cost("instances"), Some(ApplyGroup::Structural));
    }

    #[test]
    fn structural_fields_are_not_editable_and_the_rest_are() {
        let pane = ConfigPane::sheep(web());
        for key in ["name", "instances"] {
            assert!(!pane.fields().by_key(key).unwrap().editable, "{key}");
        }
        assert!(pane.fields().by_key("max_restarts").unwrap().editable);
    }

    #[test]
    fn the_menu_counts_the_parked_fields_once() {
        let mut config = AppConfig {
            name: "web".into(),
            ..AppConfig::default()
        };
        config.env.insert("DB_HOST".into(), "db.internal".into());
        let view =
            SheepConfigView::new(config, Vec::new(), vec!["env".into(), "kill_signal".into()]);
        assert_eq!(ConfigPane::sheep(view).parked_count(), 2);
        assert_eq!(ConfigPane::sheep(web_with_args(&[])).parked_count(), 0);
    }

    #[test]
    fn a_probe_without_reuse_port_is_the_only_serial_reload() {
        assert_eq!(reload_mode(false, true, false), ReloadKind::Serial);
        assert_eq!(reload_mode(true, true, false), ReloadKind::Overlap);
        assert_eq!(reload_mode(false, true, true), ReloadKind::Overlap);
        assert_eq!(reload_mode(false, false, false), ReloadKind::Overlap);
    }

    #[test]
    fn a_pane_reads_its_reload_kind_off_the_three_fields_it_turns_on() {
        let probed = |wait_ready, reuse_port| {
            let config = AppConfig {
                name: "web".into(),
                wait_ready,
                reuse_port,
                readiness_probe: Some(ProbeConfig {
                    kind: ProbeKind::Tcp,
                    target: "127.0.0.1:8080".into(),
                    interval: UpDuration::from_millis(10_000),
                    timeout: UpDuration::from_millis(5_000),
                    failure_threshold: 3,
                }),
                ..AppConfig::default()
            };
            ConfigPane::sheep(SheepConfigView::new(config, Vec::new(), Vec::new())).reload_kind()
        };
        assert_eq!(probed(false, false), ReloadKind::Serial);
        assert_eq!(probed(true, false), ReloadKind::Overlap);
        assert_eq!(probed(false, true), ReloadKind::Overlap);
        assert_eq!(
            ConfigPane::sheep(web()).reload_kind(),
            ReloadKind::Overlap,
            "no probe, so nothing an outgoing instance could answer"
        );
    }

    #[test]
    fn overridden_and_pending_are_read_off_the_view() {
        let pane = ConfigPane::sheep(web());
        assert!(pane.is_overridden("max_restarts"));
        assert!(!pane.is_overridden("autorestart"));
        assert!(pane.is_pending("env"));
        assert!(!pane.is_pending("max_restarts"));
    }

    #[test]
    fn a_bare_up_duration_shows_its_resolved_unit_and_a_suffixed_one_is_unchanged() {
        let pane = ConfigPane::sheep(web());
        assert_eq!(pane.display_value("kill_timeout"), "1600ms");
        assert_eq!(pane.value("kill_timeout"), "1600");
        assert_eq!(pane.display_value("listen_timeout"), "3s");
    }

    /// A dog's values come from the section its operator wrote, so a
    /// spelling that `UpDuration::Display` would canonicalize to `1m`
    /// stays as `60s`. A sheep's arrive already canonicalized, `UpDuration`
    /// serializing through its own `Display`, so this is the pane that can
    /// tell the difference.
    #[test]
    fn a_dogs_own_spelling_of_a_duration_survives_the_pane() {
        let schema = serde_json::json!({
            "properties": {
                "poll": { "type": "string", "$ref": "#/$defs/UpDuration" }
            },
            "$defs": { "UpDuration": { "type": "string" } }
        });
        let pane = ConfigPane::dog("bark".into(), None, schema, "poll = \"60s\"\n".into());
        assert_eq!(pane.display_value("poll"), "60s");
    }

    /// 64 bytes is not a whole `K`/`M`/`G`, so `MemSize::Display` prints
    /// the bare digits `"64"`; 512 mebibytes already prints with a unit.
    #[test]
    fn a_bare_mem_size_shows_its_resolved_unit_and_a_suffixed_one_is_unchanged() {
        for (bytes, want) in [(64, "64 B"), (512 << 20, "512M")] {
            let config = AppConfig {
                name: "web".into(),
                max_memory: Some(MemSize::from_bytes(bytes)),
                ..AppConfig::default()
            };
            let pane = ConfigPane::sheep(SheepConfigView::new(config, Vec::new(), Vec::new()));
            assert_eq!(pane.display_value("max_memory"), want, "{bytes} bytes");
        }
    }

    /// A field with no `MemSize`/`UpDuration` grammar is untouched by
    /// `display_value`, the same as `value`.
    #[test]
    fn a_field_with_no_unit_grammar_displays_the_same_either_way() {
        let pane = ConfigPane::sheep(web());
        assert_eq!(pane.display_value("cwd"), pane.value("cwd"));
        assert_eq!(
            pane.display_value("max_restarts"),
            pane.value("max_restarts")
        );
    }

    /// A dog's schema carries no group, so the pane draws no headers, and
    /// shep does not classify a dog's field cost: `cost` answers `None`
    /// for one.
    #[test]
    fn a_dog_pane_is_flat_in_schema_order_and_marks_the_secret() {
        let pane = bark_pane();
        assert!(
            pane.fields()
                .fields()
                .iter()
                .all(|field| field.group.is_none()),
            "a dog's schema carries no group, so the pane draws no headers"
        );
        let keys: Vec<&str> = pane
            .fields()
            .fields()
            .iter()
            .map(|f| f.key.as_str())
            .collect();
        assert_eq!(
            keys,
            ["history_bytes", "poll", "rules", "sink_timeout", "sinks"],
            "schema order, which for a serde_json map is alphabetical"
        );
        assert!(
            pane.fields()
                .by_key("sinks")
                .expect("sinks is a field")
                .secret
        );
        assert_eq!(pane.value("poll"), "60s");
        assert_eq!(pane.cost("poll"), None);
    }

    /// The sheep's `dogs` row opens a screen, so neither `space` nor `d`
    /// should tell the operator to go and edit a Flockfile instead.
    #[test]
    fn a_sheeps_dogs_row_opens_a_screen_rather_than_lacking_a_widget() {
        let pane = ConfigPane::sheep(web());
        assert_eq!(pane.lock("dogs"), Some(Lock::SubScreen));
        assert_eq!(pane.lock("args"), None);
    }

    /// Shep writes a `sinks` table happily; this screen simply has no
    /// widget for a table of tables.
    #[test]
    fn a_dogs_map_field_is_locked_for_want_of_a_widget_and_not_refused() {
        let pane = bark_pane();
        assert_eq!(pane.lock("sinks"), Some(Lock::NoWidget));
        assert_eq!(pane.lock("rules"), Some(Lock::NoWidget));
        assert_eq!(pane.lock("poll"), None);
    }

    /// A dog's list is edited on the same sub-screen a sheep's `args` is, so
    /// its row is not locked and the sub-screen opens on the elements the
    /// section holds.
    #[test]
    fn a_dogs_string_list_opens_the_sub_screen_on_its_stored_elements() {
        let schema = serde_json::json!({
            "properties": {
                "hosts": { "type": "array", "items": { "type": "string" } },
            },
        });
        let mut pane = ConfigPane::dog(
            "watch".into(),
            None,
            schema,
            "hosts = [\"a\", \"b\"]\n".to_owned(),
        );
        assert!(pane.fields().by_key("hosts").expect("a field").editable);
        assert_eq!(pane.lock("hosts"), None);
        pane.open_list();
        let list = pane.list().expect("the sub-screen opens");
        assert_eq!(list.key(), "hosts");
        assert_eq!(list.elements(), ["a", "b"]);
    }

    /// A map has no widget, and a secret list would draw its elements
    /// unmasked on the sub-screen, so both stay locked in a dog pane.
    #[test]
    fn a_dogs_secret_list_and_map_stay_locked() {
        let schema = serde_json::json!({
            "properties": {
                "keys": {
                    "type": "array",
                    "items": { "type": "string" },
                    "x-shep-secret": true,
                },
                "labels": {
                    "type": "object",
                    "additionalProperties": { "type": "string" },
                },
            },
        });
        let mut pane = ConfigPane::dog(
            "watch".into(),
            None,
            schema,
            "keys = [\"sk-live\"]\n".to_owned(),
        );
        for key in ["keys", "labels"] {
            assert_eq!(pane.lock(key), Some(Lock::NoWidget), "{key}");
        }
        pane.move_to_key("keys");
        pane.open_list();
        assert!(pane.list().is_none(), "no sub-screen over a secret list");
    }

    /// A dog's own schema nests a table inside a table the same way its
    /// per-sheep one does, and `dogs.toml` spells the same data three ways:
    /// a `[models.worker]` header, dotted keys, and an inline table.
    #[test]
    fn a_nested_table_in_a_dogs_own_schema_becomes_dotted_rows() {
        let schema = serde_json::json!({
            "$ref": "#/$defs/Settings",
            "$defs": {
                "Settings": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "models": { "$ref": "#/$defs/Models" },
                    },
                },
                "Models": {
                    "type": "object",
                    "properties": { "worker": { "$ref": "#/$defs/Worker" } },
                },
                "Worker": {
                    "type": "object",
                    "properties": {
                        "model": { "type": "string" },
                        "token": { "type": "string", "x-shep-secret": true },
                    },
                },
            },
        });
        for section in [
            "[models.worker]\nmodel = \"gpt\"\ntoken = \"sk-live-51Hx9Qa\"\n",
            "models.worker.model = \"gpt\"\nmodels.worker.token = \"sk-live-51Hx9Qa\"\n",
            "models = { worker = { model = \"gpt\", token = \"sk-live-51Hx9Qa\" } }\n",
        ] {
            let pane = ConfigPane::dog("jobs".into(), None, schema.clone(), section.to_owned());
            let keys: Vec<&str> = pane
                .fields()
                .fields()
                .iter()
                .map(|f| f.key.as_str())
                .collect();
            assert_eq!(
                keys,
                ["models.worker.model", "models.worker.token", "name"],
                "{section}"
            );
            assert_eq!(pane.value("models.worker.model"), "gpt", "{section}");
            assert_eq!(
                pane.value("models.worker.token"),
                "sk-live-51Hx9Qa",
                "{section}"
            );
            assert!(
                pane.fields()
                    .by_key("models.worker.token")
                    .expect("a row for the secret")
                    .secret,
                "a secret one table down is still marked"
            );
            assert_eq!(pane.lock("models.worker.model"), None, "{section}");
        }
    }

    /// A secret row seeds its editor empty, however deep in the section the
    /// credential sits: the pane renders `<set>` and must not hand the old
    /// value back for a backspace to edit.
    #[test]
    fn a_secret_under_a_nested_table_seeds_its_editor_empty() {
        let schema = serde_json::json!({
            "properties": { "db": { "$ref": "#/$defs/Db" } },
            "$defs": {
                "Db": {
                    "type": "object",
                    "properties": {
                        "host": { "type": "string" },
                        "password": { "type": "string", "x-shep-secret": true },
                    },
                },
            },
        });
        let mut pane = ConfigPane::dog(
            "watch".into(),
            None,
            schema,
            "[db]\nhost = \"h\"\npassword = \"ab12cd34\"\n".to_owned(),
        );
        pane.move_to_key("db.password");
        pane.begin_typing();
        assert_eq!(
            pane.typing().expect("the editor is open").buffer.as_str(),
            "",
            "a secret seeds empty, at any depth"
        );
        assert!(!pane.fields().by_key("db.host").expect("a row").secret);
    }

    /// A dog's section is the single most credential-dense thing this
    /// screen loads: bark's own `sinks` table is a webhook URL with a
    /// bearer token in it. The redaction is on the pane, not on each
    /// type that carries the text.
    #[test]
    fn a_dog_panes_debug_names_no_section() {
        let pane = bark_pane();
        assert_eq!(
            format!("{pane:?}"),
            r#"ConfigPane { target: Dog { name: "bark", adopted_path: None }, fields: 5, env_keys: 0, cursor: 0 }"#
        );
    }

    /// `args` and `cwd` live in `values` and routinely carry a token or a
    /// home directory (IR-41).
    #[test]
    fn the_panes_debug_names_no_value_it_holds() {
        let mut config = AppConfig {
            name: "web".into(),
            cwd: Some("/home/ada/secret-project".into()),
            args: vec!["--token".into(), "hunter2".into()],
            ..AppConfig::default()
        };
        config.env.insert("DB_PASSWORD".into(), "hunter2".into());
        let pane = ConfigPane::sheep(SheepConfigView::new(config, Vec::new(), Vec::new()));
        assert_eq!(
            format!("{pane:?}"),
            r#"ConfigPane { target: Sheep { name: "web" }, fields: 44, env_keys: 1, cursor: 0 }"#
        );
    }
}
