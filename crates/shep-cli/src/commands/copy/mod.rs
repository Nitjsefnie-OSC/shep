//! `shep copy`: one setting, or one dog's whole table, from one sheep onto
//! another.
//!
//! Three requests do the talking. `Request::ListFlock` resolves the two names
//! against the flock, `Request::SheepConfig` reads each side, and the write is
//! the request the plan turns into: [`Request::SetSheepField`] for a field,
//! [`Request::SetSheepDogSettings`] for a dog's table. There is no copy
//! request, so everything those refuse this refuses, with the same reason.
//!
//! The order is resolve, read, plan, print, ask, write. Nothing is sent until
//! the operator has seen every key the copy touches, which is also why a
//! refusal never leaves a half-written sheep behind.
//!
//! The table form prints the change list before the question, because that is
//! what the question is asked beside. The JSON form prints one envelope at the
//! end, carrying the same list beside `written` and `pending`: one object per
//! invocation is the rule `--format json` keeps.

pub(crate) mod address;
pub(crate) mod plan;

use std::fmt;
use std::io::{BufRead, IsTerminal as _};

use shep_client::Client;
use shep_core::protocol::{Request, Response};

use crate::cli::{CopyArgs, Format};
use crate::commands::rpc::{client_error, unexpected_response};
use crate::exit::ExitCode;
use crate::output::{CopyChange, CopyReport, Streams, emit, write_outcome};

use address::{Address, ConfigPath, FlockNames};
use plan::{Plan, Write};

/// Why a copy stopped before it wrote anything.
///
/// Every variant carries the sentence the operator reads, because the reason
/// is the remedy: `env` names `shep import env`, an instance count names
/// `shep stock`, and a missing source names the path that is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CopyRefusal {
    /// An address or a flag is malformed, or the two sides name different
    /// paths, or the same sheep.
    Usage(String),
    /// No sheep in this flock answers that name.
    UnknownSheep(String),
    /// The source sheep has no table, or no key, at the path named.
    MissingSource(String),
    /// A key this verb never copies, in the reason the shepherd gives for it.
    Refused(String),
}

impl CopyRefusal {
    /// The sentence the operator reads.
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Usage(message)
            | Self::UnknownSheep(message)
            | Self::MissingSource(message)
            | Self::Refused(message) => message,
        }
    }

    /// The code that reports it.
    ///
    /// [`ExitCode::InvalidConfig`] for a refused key is the code the shepherd's
    /// own refusal of that key carries, so `copy` and the request it stands in
    /// for exit alike.
    const fn code(&self) -> ExitCode {
        match self {
            Self::Usage(_) => ExitCode::Usage,
            Self::UnknownSheep(_) | Self::MissingSource(_) => ExitCode::NotFound,
            Self::Refused(_) => ExitCode::InvalidConfig,
        }
    }

    /// Writes the refusal and hands back the code it wrote.
    fn report(&self, streams: &mut Streams<'_>) -> ExitCode {
        streams.fail(self.code(), self.message())
    }
}

impl fmt::Display for CopyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl core::error::Error for CopyRefusal {}

/// Whether this run writes, asks, refuses, or has nothing to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Go {
    /// Nothing differs, so there is nothing to write and nothing to ask about.
    Nothing,
    /// The operator asked for the changes and no write.
    DryRun,
    /// Write without asking.
    Write,
    /// Ask first, and write only on a yes.
    Ask,
    /// There is no terminal to ask on, and no `--yes` to skip it.
    Refuse,
}

/// What `--dry-run`, `--yes` and a terminal add up to.
fn decide(dry_run: bool, yes: bool, stdin_is_terminal: bool, has_changes: bool) -> Go {
    if !has_changes {
        return Go::Nothing;
    }
    if dry_run {
        Go::DryRun
    } else if yes {
        Go::Write
    } else if stdin_is_terminal {
        Go::Ask
    } else {
        Go::Refuse
    }
}

/// `shep copy <from> <to>`.
///
/// # Errors
/// The [`ExitCode`] already reported to `streams`: a malformed address, a name
/// no sheep answers, a key this verb refuses, a missing source, a refusal to
/// write with no terminal to ask on, or a write the shepherd refused.
pub(crate) async fn copy(client: &Client, streams: &mut Streams<'_>, args: &CopyArgs) -> ExitCode {
    let flock = match flock_names(client, streams).await {
        Ok(names) => names,
        Err(code) => return code,
    };
    let from = match address::parse(&args.from, &flock) {
        Ok(address) => address,
        Err(refusal) => return refusal.report(streams),
    };
    let to = match address::parse(&args.to, &flock) {
        Ok(address) => address,
        Err(refusal) => return refusal.report(streams),
    };
    if let Err(refusal) = same_pair(&from, &to) {
        return refusal.report(streams);
    }
    let except = match plan::except_paths(&args.except) {
        Ok(paths) => paths,
        Err(refusal) => return refusal.report(streams),
    };

    let source = match read_config(client, streams, &from.sheep).await {
        Ok(config) => config,
        Err(code) => return code,
    };
    let target = match read_config(client, streams, &to.sheep).await {
        Ok(config) => config,
        Err(code) => return code,
    };
    let plan = match build_plan(&from, &except, args.replace, &source, &target) {
        Ok(plan) => plan,
        Err(refusal) => return refusal.report(streams),
    };

    let path = from.path.dotted();
    let mut report = report_for(&from, &to, &plan, false, args.dry_run);

    if plan.changes.is_empty() {
        let message = format!(
            "nothing to copy: `{}` already holds the same value at `{path}`",
            to.sheep
        );
        streams.note("copy", &message);
        return finish(streams, &report);
    }

    // The table form shows the change list before it asks anything about it.
    if streams.fmt == Format::Table
        && let Err(code) = print_changes(streams, &report)
    {
        return code;
    }

    let go = decide(
        args.dry_run,
        args.yes,
        std::io::stdin().is_terminal(),
        !plan.changes.is_empty(),
    );
    if go == Go::Refuse {
        return streams.fail(
            ExitCode::Usage,
            "nothing was written: `shep copy` asks before it writes, and this run has no \
             terminal to ask on. Pass --yes to write without asking, or --dry-run to see the \
             changes and write nothing",
        );
    }
    if go == Go::Ask && !confirm(streams, &from.sheep, &to.sheep, &path, plan.changes.len()) {
        streams.note("copy", "nothing was written");
        return finish(streams, &report);
    }
    if go == Go::DryRun {
        streams.note("dry_run", "nothing was written");
        return finish(streams, &report);
    }

    let Some(write) = plan.write.clone() else {
        let message = format!("the plan named {} changes and no write", plan.changes.len());
        return streams.fail(ExitCode::Internal, &message);
    };
    let pending = match write_plan(client, streams, &to.sheep, write).await {
        Ok(pending) => pending,
        Err(code) => return code,
    };

    report = report_for(&from, &to, &plan, true, args.dry_run);
    if !pending.is_empty() {
        let message = format!(
            "{} waiting on the next spawn (`shep reload {}` promotes them)",
            pending.join(", "),
            to.sheep
        );
        streams.note("parked", &message);
    }
    report.pending = pending;
    if streams.fmt == Format::Table {
        let message = format!(
            "wrote {} change{} onto `{}`",
            plan.changes.len(),
            if plan.changes.len() == 1 { "" } else { "s" },
            to.sheep
        );
        streams.note("written", &message);
    }
    finish(streams, &report)
}

/// The flock's names, split into sheep and dogs.
///
/// # Errors
/// The [`ExitCode`] already reported to `streams`.
async fn flock_names(client: &Client, streams: &mut Streams<'_>) -> Result<FlockNames, ExitCode> {
    match client.request(Request::ListFlock).await {
        Ok(Response::Flock(info)) => Ok(FlockNames {
            sheep: info
                .iter()
                .filter(|info| info.dog.is_none())
                .map(|info| info.name.clone())
                .collect(),
            dogs: info
                .iter()
                .filter(|info| info.dog.is_some())
                .map(|info| info.name.clone())
                .collect(),
        }),
        Ok(_other) => Err(unexpected_response(streams)),
        Err(err) => Err(client_error(streams, &err)),
    }
}

/// One sheep's config as JSON, which is the shape a plan reads.
///
/// `env` is already emptied by the shepherd, which is what makes `env` a
/// refusal here rather than a copy of something that is not there.
///
/// # Errors
/// The [`ExitCode`] already reported to `streams`.
async fn read_config(
    client: &Client,
    streams: &mut Streams<'_>,
    name: &str,
) -> Result<serde_json::Value, ExitCode> {
    match client
        .request(Request::SheepConfig {
            name: name.to_string(),
        })
        .await
    {
        Ok(Response::SheepConfig(view)) => serde_json::to_value(&view.config).map_err(|_| {
            streams.fail(
                ExitCode::Internal,
                "the shepherd's config answer did not serialize, so nothing was copied",
            )
        }),
        Ok(_other) => Err(unexpected_response(streams)),
        Err(err) => Err(client_error(streams, &err)),
    }
}

/// The two sides of one copy: two different sheep, and one path between them.
///
/// # Errors
/// [`CopyRefusal::Usage`] for a path that differs, or for a sheep named on
/// both sides.
fn same_pair(from: &Address, to: &Address) -> Result<(), CopyRefusal> {
    if from.path != to.path {
        return Err(CopyRefusal::Usage(format!(
            "the path must be the same on both sides: `{}.{}` and `{}.{}`",
            from.sheep,
            from.path.dotted(),
            to.sheep,
            to.path.dotted()
        )));
    }
    if from.sheep == to.sheep {
        return Err(CopyRefusal::Usage(format!(
            "`{}` is on both sides: a copy needs two different sheep",
            from.sheep
        )));
    }
    Ok(())
}

/// The plan for one copy.
///
/// # Errors
/// [`CopyRefusal::Usage`] for `--except` on a field, and whatever
/// [`plan::plan_dog`] refuses about the source.
fn build_plan(
    from: &Address,
    except: &[Vec<String>],
    replace: bool,
    source: &serde_json::Value,
    target: &serde_json::Value,
) -> Result<Plan, CopyRefusal> {
    match &from.path {
        ConfigPath::Field { name } => {
            if !except.is_empty() {
                return Err(CopyRefusal::Usage(format!(
                    "--except names a key inside a dog's table: `{name}` is one field, and a \
                     copy takes a field whole"
                )));
            }
            Ok(plan::plan_field(name, source, target))
        }
        ConfigPath::Dog { dog, keys } => {
            plan::plan_dog(&from.sheep, dog, keys, except, replace, source, target)
        }
    }
}

/// The report a plan renders, before or after the write.
fn report_for(
    from: &Address,
    to: &Address,
    plan: &Plan,
    written: bool,
    dry_run: bool,
) -> CopyReport {
    CopyReport {
        from: from.sheep.clone(),
        to: to.sheep.clone(),
        path: from.path.dotted(),
        written,
        dry_run,
        pending: Vec::new(),
        changes: plan
            .changes
            .iter()
            .map(|change| CopyChange {
                key: change.key.clone(),
                change: change.kind.as_str(),
                old: change.old.clone(),
                new: change.new.clone(),
            })
            .collect(),
    }
}

/// Prints the change list in the table form.
///
/// # Errors
/// [`ExitCode::Failure`] if the write itself failed.
fn print_changes(streams: &mut Streams<'_>, report: &CopyReport) -> Result<(), ExitCode> {
    emit(
        &mut *streams.out,
        Format::Table,
        "copy",
        report.clone(),
        streams.style,
    )
    .map_err(|_| ExitCode::Failure)
}

/// Asks the operator, and reports whether they said yes.
///
/// The question goes on stderr so a piped stdout stays what it was, and an
/// answer that is not `y` or `yes`, including no answer at all, is a no.
fn confirm(streams: &mut Streams<'_>, from: &str, to: &str, path: &str, changes: usize) -> bool {
    let question = format!(
        "copy {changes} change{} from `{from}` onto `{to}` at `{path}`? [y/N] ",
        if changes == 1 { "" } else { "s" }
    );
    streams.aside("confirm", &question);
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).unwrap_or(0) == 0 {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Sends the plan's write, and reports which keys wait for a respawn.
///
/// # Errors
/// The [`ExitCode`] already reported to `streams`.
async fn write_plan(
    client: &Client,
    streams: &mut Streams<'_>,
    target: &str,
    write: Write,
) -> Result<Vec<String>, ExitCode> {
    match write {
        Write::Field { name, value } => {
            let request = Request::SetSheepField {
                name: target.to_string(),
                key: name.clone(),
                value,
            };
            match client.request(request).await {
                Ok(Response::SheepFieldSet {
                    pending, warning, ..
                }) => {
                    if let Some(warning) = warning {
                        streams.aside("warning", &warning);
                    }
                    Ok(pending.then_some(name).into_iter().collect())
                }
                Ok(_other) => Err(unexpected_response(streams)),
                Err(err) => Err(client_error(streams, &err)),
            }
        }
        Write::DogTable { dog, table } => {
            let request = Request::SetSheepDogSettings {
                name: target.to_string(),
                dog,
                table: Some(table),
            };
            match client.request(request).await {
                Ok(Response::SheepDogSettingsSet { .. }) => Ok(Vec::new()),
                Ok(_other) => Err(unexpected_response(streams)),
                Err(err) => Err(client_error(streams, &err)),
            }
        }
    }
}

/// Prints the run's verdict.
///
/// The table form already printed the change list, so this adds nothing to it.
/// The JSON form prints the one envelope this invocation owes, with `written`
/// and `pending` filled in by what actually happened.
fn finish(streams: &mut Streams<'_>, report: &CopyReport) -> ExitCode {
    if streams.fmt != Format::Json {
        return ExitCode::Success;
    }
    write_outcome(emit(
        &mut *streams.out,
        Format::Json,
        "copy",
        report.clone(),
        streams.style,
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::commands::copy::plan::{Change, ChangeKind};
    use crate::output::Render;

    /// The flock every test here resolves against.
    fn flock() -> FlockNames {
        FlockNames {
            sheep: ["shep", "shep-kelpie", "web", "api", "we.b"]
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            dogs: ["metrics"].iter().map(|name| (*name).to_string()).collect(),
        }
    }

    /// One sheep's config, holding one dog table built from `table`.
    fn config_with_dog(dog: &str, table: serde_json::Value) -> serde_json::Value {
        json!({ "name": "web", "dogs": { dog: table } })
    }

    /// The pure half of a copy run: resolve, check the pair, plan.
    fn resolve(
        from: &str,
        to: &str,
        source: serde_json::Value,
        target: serde_json::Value,
        except: &[String],
        replace: bool,
    ) -> Result<Plan, CopyRefusal> {
        let from = address::parse(from, &flock())?;
        let to = address::parse(to, &flock())?;
        same_pair(&from, &to)?;
        let except = plan::except_paths(except)?;
        build_plan(&from, &except, replace, &source, &target)
    }

    /// The table the plan would leave on the target, for a dog copy.
    fn written_table(plan: &Plan) -> serde_json::Map<String, serde_json::Value> {
        let Write::DogTable { table, .. } = plan.write.clone().expect("a dog copy writes") else {
            panic!("a dog copy writes a dog table");
        };
        table.into_map()
    }

    #[test]
    fn a_dog_sub_table_merges_onto_the_target_and_keeps_its_other_keys() {
        let source = config_with_dog(
            "kelpie",
            json!({ "rulings": { "decide": "panel", "report": "brief" }, "git": { "remote": "origin" } }),
        );
        let target = config_with_dog(
            "kelpie",
            json!({ "rulings": { "decide": "issue", "own": "keep" }, "watch": true }),
        );
        let plan = resolve(
            "shep.dogs.kelpie.rulings",
            "shep-kelpie.dogs.kelpie.rulings",
            source,
            target,
            &[],
            false,
        )
        .expect("the source has this table");

        let kinds: Vec<(&str, &str)> = plan
            .changes
            .iter()
            .map(|change| (change.key.as_str(), change.kind.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [("decide", "changed"), ("report", "added"),],
            "the copied sub-table is what is judged, and a key only the target has inside it stays"
        );

        let table = written_table(&plan);
        assert_eq!(
            table["rulings"],
            json!({ "decide": "panel", "own": "keep", "report": "brief" }),
            "the sub-table merges: the source's keys overwrite, the target's own key stays"
        );
        assert_eq!(table["watch"], json!(true), "a key outside the path stays");
        assert!(
            !table.contains_key("git"),
            "the source's `git` sits outside the copied path, so it is not copied"
        );
    }

    #[test]
    fn replace_makes_the_target_table_exactly_the_source() {
        let source = config_with_dog("kelpie", json!({ "rulings": { "decide": "panel" } }));
        let target = config_with_dog(
            "kelpie",
            json!({ "rulings": { "decide": "issue", "own": "gone" }, "watch": true }),
        );
        let plan = resolve(
            "shep.dogs.kelpie",
            "shep-kelpie.dogs.kelpie",
            source,
            target,
            &[],
            true,
        )
        .expect("the source has this table");

        let kinds: Vec<(&str, &str)> = plan
            .changes
            .iter()
            .map(|change| (change.key.as_str(), change.kind.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("rulings.decide", "changed"),
                ("rulings.own", "removed"),
                ("watch", "removed"),
            ]
        );
        assert_eq!(
            serde_json::Value::Object(written_table(&plan)),
            json!({ "rulings": { "decide": "panel" } })
        );
    }

    #[test]
    fn an_excepted_key_is_left_as_the_target_had_it() {
        let source = config_with_dog(
            "kelpie",
            json!({ "git": { "checkout": "./src", "remote": "upstream" }, "rulings": { "decide": "panel" } }),
        );
        let target = config_with_dog(
            "kelpie",
            json!({ "git": { "checkout": "./mine", "branch": "main" } }),
        );
        let plan = resolve(
            "shep.dogs.kelpie",
            "shep-kelpie.dogs.kelpie",
            source,
            target,
            &["git.checkout".to_string()],
            false,
        )
        .expect("the source has this table");

        let keys: Vec<&str> = plan.changes.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["git.remote", "rulings.decide"]);
        assert_eq!(
            written_table(&plan)["git"],
            json!({ "branch": "main", "checkout": "./mine", "remote": "upstream" }),
            "`git.checkout` is the target's own value, and `git.branch` was never in question"
        );
    }

    #[test]
    fn a_field_copies_and_the_refused_fields_say_why() {
        let source = json!({ "name": "web", "restart_delay": "3s", "env": {} });
        let target = json!({ "name": "api", "restart_delay": "1s", "env": {} });
        let plan = resolve(
            "web.restart_delay",
            "api.restart_delay",
            source.clone(),
            target.clone(),
            &[],
            false,
        )
        .expect("restart_delay is a field");
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].kind, ChangeKind::Changed);
        assert_eq!(plan.changes[0].old, Some(json!("1s")));
        assert_eq!(
            plan.write,
            Some(Write::Field {
                name: "restart_delay".to_string(),
                value: json!("3s"),
            }),
            "the write is the request the shepherd already answers"
        );

        for (path, expected) in [
            ("web.env", "shep import env"),
            ("web.name", "different sheep"),
            ("web.instances", "shep stock web"),
            ("web.dogs", "web.dogs.<dog>"),
        ] {
            let err = resolve(
                path,
                &path.replace("web.", "api."),
                source.clone(),
                target.clone(),
                &[],
                false,
            )
            .expect_err("{path} is refused");
            assert!(err.message().contains(expected), "{path}: {err}");
        }
    }

    #[test]
    fn different_paths_on_the_two_sides_are_refused() {
        let source = config_with_dog("kelpie", json!({ "rulings": { "decide": "panel" } }));
        let target = config_with_dog("jobs", json!({ "rulings": { "decide": "issue" } }));
        let err = resolve(
            "web.dogs.kelpie.rulings",
            "api.dogs.jobs.rulings",
            source.clone(),
            target.clone(),
            &[],
            false,
        )
        .expect_err("a dog name is part of the path");
        assert_eq!(
            err,
            CopyRefusal::Usage(
                "the path must be the same on both sides: `web.dogs.kelpie.rulings` and \
                 `api.dogs.jobs.rulings`"
                    .to_string()
            )
        );

        let err = resolve(
            "web.restart_delay",
            "api.kill_timeout",
            source,
            target,
            &[],
            false,
        )
        .expect_err("two fields are two paths");
        assert!(err.message().contains("the path must be the same"), "{err}");
    }

    #[test]
    fn dry_run_writes_nothing_and_a_pipe_needs_yes() {
        assert_eq!(decide(true, false, false, true), Go::DryRun);
        assert_eq!(decide(true, true, true, true), Go::DryRun);
        assert_eq!(decide(false, false, false, true), Go::Refuse);
        assert_eq!(decide(false, true, false, true), Go::Write);
        assert_eq!(decide(false, false, true, true), Go::Ask);
        assert_eq!(decide(false, false, true, false), Go::Nothing);

        let source = config_with_dog("kelpie", json!({ "rulings": { "decide": "panel" } }));
        let target = config_with_dog("kelpie", json!({ "rulings": { "decide": "issue" } }));
        let plan = resolve(
            "web.dogs.kelpie",
            "api.dogs.kelpie",
            source,
            target,
            &[],
            false,
        )
        .expect("the source has this table");

        let dry = report_for(
            &address::parse("web.dogs.kelpie", &flock()).unwrap(),
            &address::parse("api.dogs.kelpie", &flock()).unwrap(),
            &plan,
            false,
            true,
        );
        assert!(dry.dry_run);
        assert!(
            !dry.written,
            "a dry run reports no write, whatever the plan holds"
        );
        assert_eq!(dry.changes.len(), 1);
        assert_ne!(decide(true, false, false, true), Go::Write);
        assert_ne!(decide(false, false, false, true), Go::Write);
    }

    #[test]
    fn a_missing_source_is_named() {
        let source = config_with_dog("kelpie", json!({ "rulings": { "decide": "panel" } }));
        let target = config_with_dog("kelpie", json!({ "rulings": { "decide": "issue" } }));

        let err = resolve(
            "web.dogs.jobs",
            "api.dogs.jobs",
            source.clone(),
            target.clone(),
            &[],
            false,
        )
        .expect_err("no table for `jobs`");
        assert_eq!(
            err,
            CopyRefusal::MissingSource(
                "no `web.dogs.jobs`: that sheep has no table for the dog `jobs`".to_string()
            )
        );

        let err = resolve(
            "web.dogs.kelpie.reporting",
            "api.dogs.kelpie.reporting",
            source.clone(),
            target.clone(),
            &[],
            false,
        )
        .expect_err("no key `reporting`");
        assert_eq!(
            err,
            CopyRefusal::MissingSource("no `reporting` under `web.dogs.kelpie`".to_string())
        );

        let err = resolve(
            "nope.dogs.kelpie",
            "api.dogs.kelpie",
            source,
            target,
            &[],
            false,
        )
        .expect_err("no sheep `nope`");
        assert_eq!(
            err,
            CopyRefusal::UnknownSheep(
                "no sheep named `nope`; `shep flock` lists the flock".to_string()
            )
        );
    }

    #[test]
    fn a_sheep_name_with_a_dot_resolves() {
        let source = config_with_dog("kelpie", json!({ "rulings": { "decide": "panel" } }));
        let target = config_with_dog("kelpie", json!({ "rulings": { "decide": "issue" } }));
        let plan = resolve(
            "we.b.dogs.kelpie.rulings",
            "api.dogs.kelpie.rulings",
            source,
            target,
            &[],
            false,
        )
        .expect("`we.b` is a sheep name, dots and all");
        assert_eq!(plan.changes.len(), 1);
        assert_eq!(plan.changes[0].key, "decide");
    }

    /// A copy onto itself cannot differ, and saying so beats an empty table.
    #[test]
    fn the_same_sheep_on_both_sides_is_refused() {
        let err = same_pair(
            &address::parse("web.dogs.kelpie", &flock()).unwrap(),
            &address::parse("web.dogs.kelpie", &flock()).unwrap(),
        )
        .expect_err("one sheep is not a copy");
        assert_eq!(
            err,
            CopyRefusal::Usage(
                "`web` is on both sides: a copy needs two different sheep".to_string()
            )
        );
    }

    /// `--except` names a key inside a dog's table, and a field is not one.
    #[test]
    fn except_on_a_field_is_refused() {
        let err = resolve(
            "web.restart_delay",
            "api.restart_delay",
            json!({ "restart_delay": "3s" }),
            json!({ "restart_delay": "1s" }),
            &["git.checkout".to_string()],
            false,
        )
        .expect_err("a field has no keys to leave out");
        assert!(err.message().contains("one field"), "{err}");
    }

    /// The JSON envelope carries the run's own verdict beside the change list.
    #[test]
    fn a_report_prints_its_changes_and_its_verdict() {
        let from = address::parse("web.dogs.kelpie", &flock()).unwrap();
        let to = address::parse("api.dogs.kelpie", &flock()).unwrap();
        let plan = Plan {
            changes: vec![Change {
                key: "rulings.decide".to_string(),
                kind: ChangeKind::Changed,
                old: Some(json!("issue")),
                new: Some(json!("panel")),
            }],
            write: None,
        };
        let report = report_for(&from, &to, &plan, true, false);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["from"], "web");
        assert_eq!(json["to"], "api");
        assert_eq!(json["path"], "dogs.kelpie");
        assert_eq!(json["written"], true);
        assert_eq!(json["dry_run"], false);
        assert_eq!(json["changes"][0]["key"], "rulings.decide");
        assert_eq!(json["changes"][0]["change"], "changed");
        assert_eq!(json["changes"][0]["old"], "issue");
        assert_eq!(json["changes"][0]["new"], "panel");
        assert_eq!(
            report.rows(),
            vec![vec![
                "rulings.decide".to_string(),
                "changed".to_string(),
                "issue".to_string(),
                "panel".to_string(),
            ]]
        );
    }
}
