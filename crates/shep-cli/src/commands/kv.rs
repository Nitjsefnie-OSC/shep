//! `shep kv get` / `shep kv set` / `shep kv unset`: the CLI half of
//! [`shep_core::kv`], and the hidden `shep get`, `shep set` and `shep unset`
//! that still reach the same store.
//!
//! No [`Client`](shep_client::Client) anywhere in this module. The store has
//! to work with no shepherd running, so `main` dispatches straight off the
//! resolved [`ShepPaths`] rather than through `connect_client`.

use shep_core::kv::{self, KvError};
use shep_core::paths::ShepPaths;

use crate::cli::{KvGetArgs, KvSetArgs, KvUnsetArgs};
use crate::exit::ExitCode;
use crate::output::{KvEntry, KvRows, KvUnsetRow, Streams, emit, write_outcome};

/// The notice code a hidden alias prints on stderr.
const MOVED_NOTICE: &str = "moved";

/// Which spelling reached the store.
///
/// The two differ in nothing but the `command` key their envelope carries and
/// whether the run says out loud that the verb moved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KvVia {
    /// `shep kv get`/`set`/`unset`: the store's own home.
    Kv,
    /// The hidden `shep get`/`set`/`unset`, carrying the verb as typed, which
    /// is the `command` key the scripts written against it already read.
    Alias(&'static str),
}

impl KvVia {
    /// The `command` key this spelling's envelope carries.
    fn command(self) -> &'static str {
        match self {
            Self::Kv => "kv",
            Self::Alias(verb) => verb,
        }
    }

    /// Prints the move notice on stderr for a hidden alias, and does nothing
    /// for `shep kv`.
    ///
    /// Stderr, so a `--format json` run on stdout is byte-identical to the one
    /// an operator gets from `shep kv`.
    fn announce(self, streams: &mut Streams<'_>) {
        let Self::Alias(verb) = self else {
            return;
        };
        streams.aside(
            MOVED_NOTICE,
            &format!("`shep {verb}` moved to `shep kv {verb}`; the old spelling still works"),
        );
    }
}

/// The exit code each [`KvError`] maps to.
///
/// `InvalidKey`/`ValueTooLong` are `Usage`: the operator typed it.
/// `FutureVersion`/`Decode` are `InvalidConfig`: the file on disk is the
/// problem. `KvError` is `#[non_exhaustive]`, so a future variant falls
/// through to [`ExitCode::Failure`].
fn exit_code_for(err: &KvError) -> ExitCode {
    match err {
        KvError::InvalidKey(_) | KvError::ValueTooLong { .. } => ExitCode::Usage,
        KvError::FutureVersion(_) | KvError::Decode(_) => ExitCode::InvalidConfig,
        // `KvError::Io` and any future variant both land here.
        _ => ExitCode::Failure,
    }
}

/// Renders `err` to `streams.err` and returns the code [`exit_code_for`]
/// maps it to.
fn fail(streams: &mut Streams<'_>, err: &KvError) -> ExitCode {
    let code = exit_code_for(err);
    streams.fail(code, &err.to_string())
}

/// `shep kv set <key> <value>`, and the hidden `shep set <key> <value>`.
pub fn set(streams: &mut Streams<'_>, paths: &ShepPaths, args: &KvSetArgs, via: KvVia) -> ExitCode {
    via.announce(streams);
    match kv::set(&paths.kv, &args.key, &args.value) {
        Ok(()) => {
            let row = KvRows(vec![KvEntry {
                key: args.key.clone(),
                value: args.value.clone(),
            }]);
            write_outcome(emit(
                &mut *streams.out,
                streams.fmt,
                via.command(),
                row,
                streams.style,
            ))
        }
        Err(err) => fail(streams, &err),
    }
}

/// `shep kv get [key]`, and the hidden `shep get [key]`: one value, or with no
/// key the whole store in [`kv::all`]'s `BTreeMap` order.
///
/// Exits [`ExitCode::NotFound`] for a key the store does not have, writing
/// nothing to `streams.out`, so `shep kv get k || echo default` works in a
/// script.
pub fn get(streams: &mut Streams<'_>, paths: &ShepPaths, args: &KvGetArgs, via: KvVia) -> ExitCode {
    via.announce(streams);
    let Some(key) = &args.key else {
        return match kv::all(&paths.kv) {
            Ok(entries) => {
                let rows = KvRows(
                    entries
                        .into_iter()
                        .map(|(key, value)| KvEntry { key, value })
                        .collect(),
                );
                write_outcome(emit(
                    &mut *streams.out,
                    streams.fmt,
                    via.command(),
                    rows,
                    streams.style,
                ))
            }
            Err(err) => fail(streams, &err),
        };
    };

    match kv::get(&paths.kv, key) {
        Ok(Some(value)) => {
            let row = KvRows(vec![KvEntry {
                key: key.clone(),
                value,
            }]);
            write_outcome(emit(
                &mut *streams.out,
                streams.fmt,
                via.command(),
                row,
                streams.style,
            ))
        }
        Ok(None) => {
            let message = format!("`{key}` is not set");
            streams.fail(ExitCode::NotFound, &message)
        }
        Err(err) => fail(streams, &err),
    }
}

/// `shep kv unset <key>` / `shep kv unset --all`, and the hidden `shep unset`
/// spellings of both.
///
/// Exits [`ExitCode::NotFound`] for a key the store does not have, rather
/// than exiting 0 on a no-op an operator would read as success.
pub fn unset(
    streams: &mut Streams<'_>,
    paths: &ShepPaths,
    args: &KvUnsetArgs,
    via: KvVia,
) -> ExitCode {
    via.announce(streams);
    if args.all {
        return match kv::clear(&paths.kv) {
            Ok(removed) => write_outcome(emit(
                &mut *streams.out,
                streams.fmt,
                via.command(),
                KvUnsetRow { removed },
                streams.style,
            )),
            Err(err) => fail(streams, &err),
        };
    }

    // clap's `required_unless_present = "all"` on `KvUnsetArgs` guarantees
    // `key.is_some()` here: the `args.all` arm above already returned.
    let key = args
        .key
        .as_deref()
        .expect("clap requires a key when --all is not set");
    match kv::unset(&paths.kv, key) {
        Ok(true) => write_outcome(emit(
            &mut *streams.out,
            streams.fmt,
            via.command(),
            KvUnsetRow { removed: 1 },
            streams.style,
        )),
        Ok(false) => {
            let message = format!("`{key}` is not set");
            streams.fail(ExitCode::NotFound, &message)
        }
        Err(err) => fail(streams, &err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Format;

    fn streams<'a>(out: &'a mut Vec<u8>, err: &'a mut Vec<u8>) -> Streams<'a> {
        Streams {
            out,
            err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Json,
        }
    }

    /// `$SHEP_HOME` is `dir` itself, so `paths.kv`'s parent exists:
    /// `kv::set` stages via `tempfile_in`, which creates no parent.
    fn kv_path(dir: &tempfile::TempDir) -> ShepPaths {
        let home = dir.path().display().to_string();
        shep_core::paths::ShepPaths::resolve(
            &move |key| (key == "SHEP_HOME").then(|| home.clone()),
            dir.path(),
        )
    }

    #[test]
    fn set_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = set(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvSetArgs {
                key: "bark.cooldown".to_string(),
                value: "30s".to_string(),
            },
            KvVia::Kv,
        );
        assert_eq!(code, ExitCode::Success);

        out.clear();
        let code = get(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvGetArgs {
                key: Some("bark.cooldown".to_string()),
            },
            KvVia::Kv,
        );
        assert_eq!(code, ExitCode::Success);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("bark.cooldown"), "{text}");
        assert!(text.contains("30s"), "{text}");
    }

    #[test]
    fn get_on_an_absent_key_exits_not_found_and_writes_nothing_to_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = get(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvGetArgs {
                key: Some("ghost".to_string()),
            },
            KvVia::Kv,
        );
        assert_eq!(code, ExitCode::NotFound);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn unset_all_empties_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);
        let mut out = Vec::new();
        let mut err = Vec::new();
        set(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvSetArgs {
                key: "a".to_string(),
                value: "1".to_string(),
            },
            KvVia::Kv,
        );
        set(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvSetArgs {
                key: "b".to_string(),
                value: "2".to_string(),
            },
            KvVia::Kv,
        );

        out.clear();
        let code = unset(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvUnsetArgs {
                key: None,
                all: true,
            },
            KvVia::Kv,
        );
        assert_eq!(code, ExitCode::Success);
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\"removed\":2"), "{text}");
        assert!(kv::all(&paths.kv).unwrap().is_empty());
    }

    #[test]
    fn a_bad_key_exits_usage_without_creating_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = set(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvSetArgs {
                key: "not valid".to_string(),
                value: "1".to_string(),
            },
            KvVia::Kv,
        );
        assert_eq!(code, ExitCode::Usage);
        assert!(
            !paths.kv.exists(),
            "the store must not be created on a refused key"
        );
    }

    /// The `command` key is what a `--format json` script keys on, so each
    /// spelling answers under the name it was called by: `kv` for the store's
    /// own home, and the verb as typed for the hidden alias, which is the name
    /// the scripts written against it already read.
    #[test]
    fn each_spelling_answers_under_its_own_command_name() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);

        for (via, expected) in [
            (KvVia::Kv, "kv"),
            (KvVia::Alias("set"), "set"),
            (KvVia::Alias("get"), "get"),
            (KvVia::Alias("unset"), "unset"),
        ] {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let args = KvSetArgs {
                key: "a".to_string(),
                value: "1".to_string(),
            };
            let code = match via {
                KvVia::Alias("get") => get(
                    &mut streams(&mut out, &mut err),
                    &paths,
                    &KvGetArgs { key: None },
                    via,
                ),
                KvVia::Alias("unset") => unset(
                    &mut streams(&mut out, &mut err),
                    &paths,
                    &KvUnsetArgs {
                        key: Some("a".to_string()),
                        all: false,
                    },
                    via,
                ),
                _ => set(&mut streams(&mut out, &mut err), &paths, &args, via),
            };
            assert_eq!(code, ExitCode::Success, "{via:?}");
            let envelope: serde_json::Value = serde_json::from_slice(&out).unwrap();
            assert_eq!(envelope["command"], expected, "{via:?}: {envelope}");
        }
    }

    /// fails if a hidden alias goes quiet, or writes its note into stdout where
    /// it would land inside the `--format json` envelope an operator is piping
    /// to `jq`.
    #[test]
    fn a_hidden_alias_names_the_new_verb_on_stderr_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = set(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvSetArgs {
                key: "bark.cooldown".to_string(),
                value: "30s".to_string(),
            },
            KvVia::Alias("set"),
        );
        assert_eq!(code, ExitCode::Success);

        let note = String::from_utf8(err).unwrap();
        assert!(note.contains("`shep set` moved to `shep kv set`"), "{note}");
        let notice: serde_json::Value = serde_json::from_slice(note.as_bytes()).unwrap();
        assert_eq!(notice["notice"]["code"], MOVED_NOTICE, "{notice}");

        // stdout is one envelope and nothing else: the note is not in it.
        let envelope: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(envelope["command"], "set", "{envelope}");
        assert!(envelope.get("notice").is_none(), "{envelope}");
    }

    /// fails if `shep kv` starts printing the note too: the move is news about
    /// the old spelling, and a run at the store's own home has nothing to be
    /// told.
    #[test]
    fn the_store_is_quiet_at_its_new_home() {
        let dir = tempfile::tempdir().unwrap();
        let paths = kv_path(&dir);
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = set(
            &mut streams(&mut out, &mut err),
            &paths,
            &KvSetArgs {
                key: "a".to_string(),
                value: "1".to_string(),
            },
            KvVia::Kv,
        );
        assert_eq!(code, ExitCode::Success);
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    }
}
