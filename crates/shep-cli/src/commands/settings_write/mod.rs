//! `shep set <address> <value>`: one write for any settings in the flock.
//!
//! The address is `shep copy`'s and `shep get`'s, resolved against the names
//! the running flock carries: a sheep's name addresses its config and the
//! `dogs.<dog>` tables inside it, and a dog's name addresses its own section
//! of `dogs.toml`. One verb reaches all three because all three are the same
//! thing to the shepherd, and each is written through the request the
//! shepherd already answers: [`Request::SetSheepField`] for a field,
//! [`Request::SetSheepDogSettings`] for a dog's table on a sheep, and
//! [`Request::SetDogConfig`] for a dog's own section. There is no set request,
//! so everything those refuse this refuses, with the same reason.
//!
//! The order is `copy`'s: resolve, read, plan, print the change list with old
//! and new values, ask, write. Nothing is sent until the operator has seen
//! every key the write touches, which is also why a refusal never leaves a
//! half-written sheep behind.
//!
//! The value is TOML, and text that is not TOML is the string as typed: `5`
//! is a number, `true` is a toggle, `["a", "b"]` is a list, `{ keep = 1 }` is
//! a table, and `3s` stays `3s`. That is the grammar an operator already
//! writes these settings in, in the file they live in.
//!
//! A write that carries a dog's table is checked against the schema that dog
//! published before anything is sent, through the same door lookout's config
//! pane files a keystroke through ([`crate::dog_settings::table`]). A value
//! the schema turns down is refused with the key path it sits at, and nothing
//! is written. A sheep's own field is the one write with no dog schema to ask,
//! so the shepherd's own validation is the gate for it, as it is for `copy`.
//!
//! The table form prints the change list before the question, because that is
//! what the question is asked beside. The JSON form prints one envelope at the
//! end, carrying the same list beside `written` and `pending`: one object per
//! invocation is the rule `--format json` keeps.

use std::collections::BTreeSet;
use std::fmt;
use std::io::IsTerminal as _;

use serde_json::{Map, Value as Json};
use shep_client::Client;
use shep_core::config::DogTable;
use shep_core::paths::ShepPaths;
use shep_core::protocol::{DogSectionToml, Request, Response};
use toml_edit::DocumentMut;

use self::plan::{field_changes, plan_dog_table, plan_section, schema_refusal};
use self::schema::{dog_schema, known_dogs};
use self::value::value_of;
use crate::cli::{Format, SetArgs};
use crate::commands::copy;
use crate::commands::copy::address::{self, ConfigPath, FlockNames};
use crate::commands::copy::confirm::{self, Answer, Go};
use crate::commands::copy::plan::Change;
use crate::commands::copy::report::shows_changes_first;
use crate::commands::rpc::{client_error, unexpected_response};
use crate::commands::settings_read::{self, Holder};
use crate::exit::ExitCode;
use crate::lookout::probe_sheep_dogs::sheep_schema;
use crate::output::{CopyChange, SetReport, Streams, emit, write_outcome};

mod edit;
mod plan;
mod schema;
mod value;

/// Why a write stopped before it wrote anything.
///
/// Every variant carries the sentence the operator reads, because the reason
/// is the remedy: an unknown name names `shep kv set`, `env` names `shep
/// import env`, and a schema refusal names the key path and the rule it
/// breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WriteRefusal {
    /// An address or a value is malformed for the path it was aimed at.
    Usage(String),
    /// No sheep and no dog in this flock answers that name.
    Unknown(String),
    /// A key this verb never writes, in the reason the shepherd gives for it.
    Refused(String),
    /// The dog's own schema turned the value down.
    Invalid(String),
}

impl WriteRefusal {
    /// The sentence the operator reads.
    fn message(&self) -> &str {
        match self {
            Self::Usage(message)
            | Self::Unknown(message)
            | Self::Refused(message)
            | Self::Invalid(message) => message,
        }
    }

    /// The code that reports it.
    ///
    /// [`ExitCode::InvalidConfig`] for a refused key and for a schema refusal
    /// is the code the shepherd's own refusal of the same write carries, so
    /// `set` and the request it stands in for exit alike.
    const fn code(&self) -> ExitCode {
        match self {
            Self::Usage(_) => ExitCode::Usage,
            Self::Unknown(_) => ExitCode::NotFound,
            Self::Refused(_) | Self::Invalid(_) => ExitCode::InvalidConfig,
        }
    }

    /// Writes the refusal and hands back the code it wrote.
    fn report(&self, streams: &mut Streams<'_>) -> ExitCode {
        streams.fail(self.code(), self.message())
    }
}

impl fmt::Display for WriteRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl core::error::Error for WriteRefusal {}

/// The request a plan turns into.
///
/// One per address shape, which is one per request the shepherd already
/// answers. A dog's section is the whole text rather than one key:
/// [`Request::SetDogConfig`] replaces the section, and the text is the one
/// this verb read, edited in place, so the comments the operator wrote inside
/// it survive the write.
#[derive(Debug, Clone)]
pub(super) enum Write {
    /// One `AppConfig` field, through [`Request::SetSheepField`].
    SheepField {
        /// The field name.
        key: String,
        /// The new value.
        value: Json,
    },
    /// One dog's whole table on one sheep, through
    /// [`Request::SetSheepDogSettings`].
    ///
    /// The whole table and not the one key: that request replaces a dog's
    /// table, so the merged result is what carries the keys this write left
    /// alone.
    SheepDogTable {
        /// The dog's name.
        dog: String,
        /// The table the sheep holds afterwards.
        table: DogTable,
    },
    /// One dog's whole section, through [`Request::SetDogConfig`].
    DogSection {
        /// The section text, edited in place from the text the shepherd
        /// answered.
        toml: String,
    },
}

/// What one `shep set` would write: the keys it changes, the one request that
/// writes them, and what the value has to pass before any of it is sent.
#[derive(Debug)]
pub(super) struct Plan {
    /// One entry per key the write touches, in dotted-key order.
    changes: Vec<Change>,
    write: Write,
    /// The table the write hands the shepherd, as JSON: the dog's table on the
    /// sheep, or the whole section. Empty for a sheep's own field, which is
    /// one value and not a table.
    table: Map<String, Json>,
    /// The schema `table` is checked against, `None` when no dog published one
    /// for it.
    schema: Option<Json>,
    /// The property paths this run chose, under `table`. Empty says the write
    /// replaces the table whole, so every key in it is in play.
    touched: BTreeSet<Vec<String>>,
}

/// What one run carries to whichever holder the address names: the operator's
/// address and value, and the flock and the files the address was resolved
/// against.
pub(super) struct Ask<'a> {
    /// The flags the operator typed: `--yes`, `--dry-run`, `--format`.
    args: &'a SetArgs,
    /// The value, already read as TOML.
    value: Json,
    /// The names the running flock carries.
    flock: &'a FlockNames,
    /// `$SHEP_HOME`, for a dog's schema and `shep.toml`'s dog list.
    paths: &'a ShepPaths,
}

/// `shep set <address> <value>`.
///
/// # Errors
/// The [`ExitCode`] already reported to `streams`: a malformed address, a name
/// no sheep or dog answers, a key this verb never writes, a value the dog's
/// schema turns down, a refusal to write with no terminal to ask on, or a
/// write the shepherd refused.
pub(crate) async fn set(
    client: &Client,
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    args: &SetArgs,
) -> ExitCode {
    let value = value_of(&args.value);
    let flock = match copy::flock_names(client, streams).await {
        Ok(names) => names,
        Err(code) => return code,
    };
    let segments = match settings_read::segments(&args.address) {
        Ok(segments) => segments,
        Err(refusal) => return refusal.report(streams),
    };
    let ask = Ask {
        args,
        value,
        flock: &flock,
        paths,
    };
    if let Some(name) = address::longest_in(&segments, &flock.sheep) {
        return set_sheep(client, streams, &name, &segments, &ask).await;
    }
    set_dog(client, streams, &segments, &ask).await
}

/// One sheep's field, or one key inside one of its dog tables.
async fn set_sheep(
    client: &Client,
    streams: &mut Streams<'_>,
    name: &str,
    segments: &[String],
    ask: &Ask<'_>,
) -> ExitCode {
    let config = match copy::read_config(client, streams, name).await {
        Ok(config) => config,
        Err(code) => return code,
    };
    let depth = name.split('.').count();
    let written = address::Written {
        sheep: name.to_string(),
        segments: segments[depth..].to_vec(),
    };
    // The same sheep on both sides of `path_of`: `copy` resolves a dotted dog
    // name against the two tables it is copying between, and this verb has the
    // one it is writing.
    let dogs = address::DogNames::new(ask.flock, &config, &Json::Null);
    let path = match address::path_of(&written, &dogs, &config, &config) {
        Ok(path) => path,
        Err(refusal) => return refusal.report(streams),
    };
    let plan = match &path {
        ConfigPath::Field { name: field } => Plan {
            changes: field_changes(field, &config, &ask.value),
            write: Write::SheepField {
                key: field.clone(),
                value: ask.value.clone(),
            },
            table: Map::new(),
            schema: None,
            touched: BTreeSet::new(),
        },
        ConfigPath::Dog { dog, keys } => {
            let plan = match plan_dog_table(name, dog, keys, &config, &ask.value) {
                Ok(plan) => plan,
                Err(refusal) => return refusal.report(streams),
            };
            // The per-sheep half of the dog's schema, the same one lookout
            // builds its rows from, and the only schema that speaks for a
            // sheep's table for a dog.
            let schema = dog_schema(dog, ask.paths).and_then(|root| sheep_schema(&root));
            Plan { schema, ..plan }
        }
    };
    run(
        client,
        streams,
        Holder::Sheep,
        name,
        &path.dotted(),
        &plan,
        ask.args,
    )
    .await
}
/// One dog's own section of `dogs.toml`.
///
/// The name is the longest run of segments the shepherd answers a section for,
/// which is what tells `kelpie` and `kelpie.worker` apart, and it is the rule
/// [`shep get`](settings_read::get) already applies: one read per candidate,
/// longest first, and the first candidate with a section is the dog.
///
/// A dog the flock runs with no section yet is the one case with nothing to
/// read, and the write creates it: the operator named a key, and a section
/// that does not exist yet is no reason to refuse it.
async fn set_dog(
    client: &Client,
    streams: &mut Streams<'_>,
    segments: &[String],
    ask: &Ask<'_>,
) -> ExitCode {
    let mut named: Option<String> = None;
    let known = known_dogs(ask.paths);
    for name in settings_read::dog_candidates(segments) {
        let section = match settings_read::read_dog_section(client, streams, &name).await {
            Ok(section) => section,
            Err(code) => return code,
        };
        if named.is_none() && (ask.flock.dogs.contains(&name) || known.contains(&name)) {
            named = Some(name.clone());
        }
        if section.trim().is_empty() {
            continue;
        }
        let Ok(doc) = section.parse::<DocumentMut>() else {
            return streams.fail(ExitCode::InvalidConfig, &unparsable(&name));
        };
        let depth = name.split('.').count();
        let rest = &segments[depth..];
        let keys = settings_read::dog_steps(doc.as_table(), rest);
        return match plan_section(doc, &keys, &ask.value) {
            Ok(plan) => {
                let plan = Plan {
                    schema: dog_schema(&name, ask.paths),
                    ..plan
                };
                run(
                    client,
                    streams,
                    Holder::Dog,
                    &name,
                    &rest.join("."),
                    &plan,
                    ask.args,
                )
                .await
            }
            Err(refusal) => refusal.report(streams),
        };
    }
    let Some(dog) = named else {
        return WriteRefusal::Unknown(no_such_name(segments)).report(streams);
    };
    let depth = dog.split('.').count();
    let rest = &segments[depth..];
    match plan_section(DocumentMut::new(), rest, &ask.value) {
        Ok(plan) => {
            let plan = Plan {
                schema: dog_schema(&dog, ask.paths),
                ..plan
            };
            run(
                client,
                streams,
                Holder::Dog,
                &dog,
                &rest.join("."),
                &plan,
                ask.args,
            )
            .await
        }
        Err(refusal) => refusal.report(streams),
    }
}
/// Prints the change list, asks, writes, and reports what the write left
/// waiting.
async fn run(
    client: &Client,
    streams: &mut Streams<'_>,
    holder: Holder,
    name: &str,
    path: &str,
    plan: &Plan,
    args: &SetArgs,
) -> ExitCode {
    if let Some(refusal) = schema_refusal(plan, name, path) {
        return refusal.report(streams);
    }
    let mut report = report_for(holder, name, path, plan, false, args.dry_run);
    if plan.changes.is_empty() {
        announce(
            streams,
            "set",
            &format!(
                "nothing to write: `{}.{}` already holds that value",
                name, report.path
            ),
        );
        return finish(streams, &report);
    }
    let go = confirm::decide(
        args.dry_run,
        args.yes,
        std::io::stdin().is_terminal(),
        !plan.changes.is_empty(),
    );

    // The change list prints before anything is asked about it, in every
    // form. Where it prints depends on what that form's stdout carries: the
    // table form's stdout is the change list, and the JSON form's is one
    // envelope that can only be written after the write the question gates.
    if shows_changes_first(streams.fmt, go)
        && let Err(code) = print_changes(streams, &report)
    {
        return code;
    }

    if go == Go::Refuse {
        return streams.fail(
            ExitCode::Usage,
            "nothing was written: `shep set` asks before it writes, and this run has no terminal \
             to ask on. Pass --yes to write without asking, or --dry-run to see the changes and \
             write nothing",
        );
    }
    if go == Go::Ask {
        let mut reader = std::io::stdin().lock();
        match ask(streams, name, &report.path, plan.changes.len(), &mut reader) {
            Answer::Yes => {}
            Answer::No => {
                announce(streams, "set", "nothing was written");
                return finish(streams, &report);
            }
            Answer::Failed(reason) => {
                return streams.fail(
                    ExitCode::Failure,
                    &format!("nothing was written: the question could not be read ({reason})"),
                );
            }
        }
    }
    if go == Go::DryRun {
        announce(streams, "dry_run", "nothing was written");
        return finish(streams, &report);
    }

    let pending = match send(client, streams, name, &plan.write).await {
        Ok(pending) => pending,
        Err(code) => return code,
    };
    report = report_for(holder, name, path, plan, true, args.dry_run);
    report.pending = pending;
    if !report.pending.is_empty() {
        announce(
            streams,
            "parked",
            &format!(
                "{} waiting on the next spawn (`shep reload {name}` promotes them)",
                report.pending.join(", ")
            ),
        );
    }
    announce(
        streams,
        "written",
        &format!(
            "wrote {} change{} onto `{}`",
            plan.changes.len(),
            if plan.changes.len() == 1 { "" } else { "s" },
            name
        ),
    );
    finish(streams, &report)
}

/// Prints a notice, in the form that has room for one.
///
/// The table form's stdout is prose lines beside the change list. The JSON
/// form's stdout is the one envelope this invocation owes, and a notice printed
/// in it is a second object on the same stream, which is what a consumer
/// piping into `jq` chokes on. Everything a notice would have said is a field
/// of that envelope: `written`, `dry_run` and `pending`.
fn announce(streams: &mut Streams<'_>, code: &str, message: &str) {
    if streams.fmt == Format::Table {
        streams.note(code, message);
    }
}

/// Asks the operator, on stderr, beside the change list the question is about.
fn ask(
    streams: &mut Streams<'_>,
    name: &str,
    path: &str,
    changes: usize,
    reader: &mut dyn std::io::BufRead,
) -> Answer {
    let question = if changes == 1 {
        format!("set `{}.{}`? [y/N] ", name, path)
    } else {
        format!("set {changes} keys under `{}.{}`? [y/N] ", name, path)
    };
    streams.aside("confirm", &question);
    let mut answer = String::new();
    match reader.read_line(&mut answer) {
        Ok(0) => Answer::No,
        Ok(_) => {
            if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                Answer::Yes
            } else {
                Answer::No
            }
        }
        Err(err) => Answer::Failed(err.to_string()),
    }
}

/// Sends the plan's write, and reports which keys wait for a respawn.
///
/// # Errors
/// The [`ExitCode`] already reported to `streams`.
async fn send(
    client: &Client,
    streams: &mut Streams<'_>,
    name: &str,
    write: &Write,
) -> Result<Vec<String>, ExitCode> {
    let request = match write {
        Write::SheepField { key, value } => Request::SetSheepField {
            name: name.to_string(),
            key: key.clone(),
            value: value.clone(),
        },
        Write::SheepDogTable { dog, table } => Request::SetSheepDogSettings {
            name: name.to_string(),
            dog: dog.clone(),
            table: Some(table.clone()),
        },
        Write::DogSection { toml } => Request::SetDogConfig {
            name: name.to_string(),
            toml: DogSectionToml::from(toml.clone()),
        },
    };
    let pending_key = match write {
        Write::SheepField { key, .. } => key.clone(),
        _ => String::new(),
    };
    match client.request(request).await {
        Ok(Response::SheepFieldSet {
            pending, warning, ..
        }) => {
            if let Some(warning) = warning {
                streams.aside("warning", &warning);
            }
            Ok(pending.then_some(pending_key).into_iter().collect())
        }
        Ok(Response::SheepDogSettingsSet { .. }) | Ok(Response::DogConfigSet { .. }) => {
            Ok(Vec::new())
        }
        Ok(_other) => Err(unexpected_response(streams)),
        Err(err) => Err(client_error(streams, &err)),
    }
}
/// The sentence for a name no sheep is and no dog has a section for, which is
/// where the key/value store `shep set` used to write went.
pub(super) fn no_such_name(segments: &[String]) -> String {
    format!(
        "no sheep or dog named `{}`: `shep flock` lists the flock and `shep dogs` lists the dogs. \
         The key/value store `shep set` used to write lives at `shep kv set`",
        segments.join(".")
    )
}

/// The sentence for a `dogs.toml` section this verb cannot read as TOML.
///
/// The file is named rather than the settings, because the file is the thing
/// that is broken: the same text [`settings_read::get`] prints is what does not
/// parse, and `shep get` is how the operator sees it.
pub(super) fn unparsable(name: &str) -> String {
    format!(
        "`{name}`'s section of `dogs.toml` does not parse: nothing was written, and `shep get \
         {name}` reads the same text"
    )
}
/// The report a plan renders, before or after the write.
pub(super) fn report_for(
    holder: Holder,
    name: &str,
    path: &str,
    plan: &Plan,
    written: bool,
    dry_run: bool,
) -> SetReport {
    SetReport {
        kind: holder.as_str(),
        name: name.to_string(),
        path: path.to_string(),
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

/// Prints the change list on the stream the operator is reading.
///
/// The table form prints it on stdout. A form whose stdout is a JSON envelope
/// prints it on stderr, so the operator sees what they are being asked to
/// approve while stdout stays the one envelope the `--format json` rule keeps.
///
/// # Errors
/// [`ExitCode::Failure`] if the write itself failed.
fn print_changes(streams: &mut Streams<'_>, report: &SetReport) -> Result<(), ExitCode> {
    let writer: &mut dyn std::io::Write = if streams.fmt == Format::Table {
        &mut *streams.out
    } else {
        &mut *streams.err
    };
    emit(writer, Format::Table, "set", report.clone(), streams.style).map_err(|_| ExitCode::Failure)
}

/// Prints the run's verdict.
///
/// The table form already printed the change list, so this adds nothing to it.
/// The JSON form prints the one envelope this invocation owes, with `written`,
/// `pending` and `dry_run` filled in by what actually happened: a parked key is
/// a field of that envelope rather than a second object on stdout.
fn finish(streams: &mut Streams<'_>, report: &SetReport) -> ExitCode {
    if streams.fmt != Format::Json {
        return ExitCode::Success;
    }
    write_outcome(emit(
        &mut *streams.out,
        Format::Json,
        "set",
        report.clone(),
        streams.style,
    ))
}

#[cfg(test)]
mod tests;
