//! The address grammar: a sheep name, then a path inside its config.
//!
//! A sheep name may itself contain a dot, so the split is not "the first
//! dot". It is the longest flock sheep name the address starts with, and
//! failing that, the first segment that is a path root: `dogs`, or a name
//! `AppConfig` has a field for. Both readings agree on every address a real
//! flock can be asked for, and a name that is neither is refused by name
//! rather than guessed at.
//!
//! A dog name may contain a dot, and so may a key inside a dog's table, and
//! neither is a segment. Each step past the sheep name therefore takes the
//! longest name the copy's tables actually hold: the flock's dogs plus the
//! tables each side already carries for the dog name, and the keys of the
//! table it is standing in. `web.dogs.foo.bar` is the dog `foo.bar` when a
//! side holds a table under that name, and the dog `foo` with the key `bar`
//! when it does not. Nothing holds either reading, the first segment stands,
//! and the plan reports the source that is not there.

use std::collections::BTreeSet;

use serde_json::{Map, Value};
use shep_core::config::{AppConfig, ApplyGroup, apply_group};

use super::CopyRefusal;

/// The one path root that is not an `AppConfig` field name.
const DOGS: &str = "dogs";

/// The flock's names, split the way an address needs them.
#[derive(Debug, Clone, Default)]
pub(crate) struct FlockNames {
    /// Every sheep in the flock, as `shep flock` spells it.
    pub(crate) sheep: BTreeSet<String>,
    /// Every dog in the flock, so a refusal can say the name is a dog's.
    pub(crate) dogs: BTreeSet<String>,
}

/// One address: a sheep, and a path inside that sheep's config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Address {
    /// The sheep the address names.
    pub(crate) sheep: String,
    /// The path inside that sheep's config.
    pub(crate) path: ConfigPath,
}

/// The path half of an [`Address`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigPath {
    /// One `AppConfig` field, such as `restart_delay`.
    Field {
        /// The field name.
        name: String,
    },
    /// One dog's table, or a key inside it. An empty `keys` is the whole
    /// table.
    Dog {
        /// The dog's name.
        dog: String,
        /// The dotted key path inside that dog's table.
        keys: Vec<String>,
    },
}

impl ConfigPath {
    /// The path as an operator writes it after the sheep name.
    pub(crate) fn dotted(&self) -> String {
        match self {
            Self::Field { name } => name.clone(),
            Self::Dog { dog, keys } => {
                let mut out = format!("{DOGS}.{dog}");
                for key in keys {
                    out.push('.');
                    out.push_str(key);
                }
                out
            }
        }
    }
}

/// Splits `raw` into a sheep name and the path segments after it.
///
/// The path half is [`path_of`], which needs the two configs the copy reads
/// before it can say where a dog name ends.
///
/// # Errors
/// [`CopyRefusal::Usage`] for an address with no sheep, no path, or an empty
/// part. [`CopyRefusal::UnknownSheep`] when no sheep in `flock` is what the
/// address starts with.
pub(crate) fn written(raw: &str, flock: &FlockNames) -> Result<Written, CopyRefusal> {
    let segments: Vec<&str> = raw.split('.').collect();
    if segments.iter().any(|part| part.is_empty()) {
        return Err(CopyRefusal::Usage(format!(
            "`{raw}` has an empty part: a sheep name and a path segment are each at least one \
             character"
        )));
    }

    match longest_sheep(&segments, flock) {
        Some(name) if name.len() == raw.len() => Err(CopyRefusal::Usage(format!(
            "`{raw}` names the sheep `{name}` and no setting: write `{name}.restart_delay`, \
             `{name}.{DOGS}.<dog>`, or `{name}.{DOGS}.<dog>.<key>`"
        ))),
        Some(name) => {
            let depth = name.split('.').count();
            Ok(Written {
                sheep: name.clone(),
                segments: segments[depth..]
                    .iter()
                    .map(|part| (*part).to_string())
                    .collect(),
            })
        }
        // No flock sheep name matches any prefix, so no reading of the
        // address can name a sheep in this flock. Refuse the longest candidate
        // the address could mean, which is what lets a name carrying a dot be
        // refused whole rather than as its first token.
        None => Err(unknown_sheep(&segments, flock)),
    }
}

/// An address as an operator wrote it: a sheep, and the segments after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Written {
    /// The sheep the address names.
    pub(crate) sheep: String,
    /// The path segments, in order, with the sheep name off the front.
    pub(crate) segments: Vec<String>,
}

/// Every dog name a copy can mean: the flock's dogs, and the tables each side
/// already holds.
///
/// A sheep can carry a table for a dog the flock has never started, so the
/// flock's list alone would read `foo.bar` as two names.
pub(crate) fn dog_names(flock: &FlockNames, source: &Value, target: &Value) -> BTreeSet<String> {
    let mut names = flock.dogs.clone();
    for config in [source, target] {
        if let Some(dogs) = dogs_of(config) {
            names.extend(dogs.keys().cloned());
        }
    }
    names
}

/// One sheep's `dogs` map, as JSON.
pub(crate) fn dogs_of(config: &Value) -> Option<&Map<String, Value>> {
    config.get(DOGS).and_then(Value::as_object)
}

/// One sheep's table for one dog.
pub(crate) fn dog_table<'a>(config: &'a Value, dog: &str) -> Option<&'a Value> {
    dogs_of(config).and_then(|dogs| dogs.get(dog))
}

/// The path half of an address, resolved against the two configs it names.
///
/// # Errors
/// [`CopyRefusal::Usage`] for an empty path, a multi-segment path that is not
/// a dog's table, or a name no field of a sheep's config is. [`CopyRefusal::Refused`]
/// for a key this verb never copies.
pub(crate) fn path_of(
    written: &Written,
    known_dogs: &BTreeSet<String>,
    source: &Value,
    target: &Value,
) -> Result<ConfigPath, CopyRefusal> {
    let sheep = written.sheep.as_str();
    let segments = written.segments.as_slice();
    let Some(first) = segments.first() else {
        return Err(CopyRefusal::Usage(format!(
            "`{sheep}` names a sheep and no setting"
        )));
    };

    if *first == DOGS {
        if segments.len() == 1 {
            return Err(CopyRefusal::Refused(format!(
                "the whole `dogs` map is not one thing to copy: name a dog, as \
                 `{sheep}.{DOGS}.<dog>`"
            )));
        }
        let rest = &segments[1..];
        let dog = longest_name(rest, known_dogs);
        let depth = dog.split('.').count();
        let keys = resolve_keys(
            &rest[depth..],
            dog_table(source, &dog),
            dog_table(target, &dog),
        );
        return Ok(ConfigPath::Dog { dog, keys });
    }

    if segments.len() > 1 {
        return Err(CopyRefusal::Usage(format!(
            "a path is one field, `{DOGS}.<dog>`, or `{DOGS}.<dog>.<key>`; `{sheep}.{}` is none \
             of those",
            segments.join(".")
        )));
    }

    let field = first.as_str();
    if field == "env" {
        return Err(CopyRefusal::Refused(
            "`env` is not copied: a client is never sent a sheep's env values, so there is \
             nothing here to read. `shep import env` writes a `.env` into a sheep's env, and \
             `shep secret` holds the values a Flockfile refers to"
                .to_string(),
        ));
    }
    if apply_group(field) == ApplyGroup::Structural {
        return Err(CopyRefusal::Refused(format!(
            "`{field}` is not a config write: a name change makes a different sheep, and \
             `shep stock {sheep} <count>` moves an instance count"
        )));
    }
    if !app_fields().any(|name| name == field) {
        return Err(CopyRefusal::Usage(format!(
            "`{field}` is not a field a sheep's config has; `shep describe {sheep}` shows the \
             config a copy reads"
        )));
    }
    Ok(ConfigPath::Field {
        name: field.to_string(),
    })
}

/// The longest name in `names` that `rest` starts with, as one dog name.
///
/// With no name to match, the first segment stands: an address naming a dog
/// nobody has a table for is a missing source the plan reports, not a parse
/// failure.
fn longest_name(rest: &[String], names: &BTreeSet<String>) -> String {
    names
        .iter()
        .filter(|name| {
            let parts: Vec<&str> = name.split('.').collect();
            parts.len() <= rest.len() && parts == rest[..parts.len()]
        })
        .max_by_key(|name| name.split('.').count())
        .cloned()
        .unwrap_or_else(|| rest[0].clone())
}

/// The key path inside a dog's table, resolved against what both sides hold.
///
/// Each step takes the longest key the tables actually hold that matches what
/// is left, so `api.token` is one key when a table holds `api.token`, and
/// `api` then `token` when it holds a table under `api`. A key nothing holds
/// stays one segment, which is what lets the plan name the source that is
/// missing rather than invent a path.
pub(crate) fn resolve_keys(
    rest: &[String],
    source: Option<&Value>,
    target: Option<&Value>,
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut src = source;
    let mut tgt = target;
    let mut next = 0;
    while next < rest.len() {
        let taken = longest_key(&rest[next..], src, tgt);
        keys.push(taken.join("."));
        for part in &taken {
            src = src.and_then(|value| value.as_object().and_then(|map| map.get(part)));
            tgt = tgt.and_then(|value| value.as_object().and_then(|map| map.get(part)));
        }
        next += taken.len();
    }
    keys
}

/// The longest run of `rest` one of the two tables holds as one key.
fn longest_key(rest: &[String], source: Option<&Value>, target: Option<&Value>) -> Vec<String> {
    for len in (1..=rest.len()).rev() {
        let name = rest[..len].join(".");
        if holds(source, &name) || holds(target, &name) {
            return rest[..len].to_vec();
        }
    }
    rest[..1].to_vec()
}

/// Whether `table` holds `key` as one key.
fn holds(table: Option<&Value>, key: &str) -> bool {
    table.is_some_and(|value| value.as_object().is_some_and(|map| map.contains_key(key)))
}

/// The longest flock sheep name `segments` starts with.
fn longest_sheep<'a>(segments: &[&'a str], flock: &'a FlockNames) -> Option<&'a String> {
    flock
        .sheep
        .iter()
        .filter(|name| {
            let parts: Vec<&str> = name.split('.').collect();
            parts.len() <= segments.len() && parts == segments[..parts.len()]
        })
        .max_by_key(|name| name.split('.').count())
}

/// The refusal for a name no sheep in the flock answers, naming it.
///
/// The candidate is the address up to its first path root, so `we.c.dogs.kelpie`
/// is refused as `we.c`.
fn unknown_sheep(segments: &[&str], flock: &FlockNames) -> CopyRefusal {
    let head = segments[0];
    if flock.dogs.contains(head) {
        return CopyRefusal::UnknownSheep(format!(
            "no sheep named `{}`: `{head}` is a dog, and a copy writes a sheep's config",
            segments.join(".")
        ));
    }
    let depth = segments
        .iter()
        .skip(1)
        .position(is_path_root)
        .map_or(segments.len(), |i| i + 1);
    CopyRefusal::UnknownSheep(format!(
        "no sheep named `{}`; `shep flock` lists the flock",
        segments[..depth].join(".")
    ))
}

/// Whether `segment` ends a sheep name and starts a path.
fn is_path_root(segment: &&str) -> bool {
    *segment == DOGS || app_fields().any(|name| name == *segment)
}

/// Every field name `AppConfig` serializes, which is every field a sheep has.
///
/// Read off the type rather than a hand-kept list, so a field added to
/// `AppConfig` is copyable without editing this file.
fn app_fields() -> impl Iterator<Item = &'static str> {
    STATIC_FIELDS.iter().map(String::as_str)
}

/// [`app_fields`] once per process: the serialization is the same every time.
static STATIC_FIELDS: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
    serde_json::to_value(AppConfig::default())
        .ok()
        .and_then(|value| value.as_object().cloned())
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
});

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The flock every test in this module resolves against.
    fn flock() -> FlockNames {
        FlockNames {
            sheep: ["web", "api", "we.b", "web.watch"]
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            dogs: ["metrics", "kelpie"]
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }

    /// Parses with no config in hand, which is every case that does not turn
    /// on what a table actually holds.
    fn parse(raw: &str, flock: &FlockNames) -> Result<Address, CopyRefusal> {
        parse_against(raw, flock, &Value::Null, &Value::Null)
    }

    /// The whole address read, against the two configs a copy would read.
    fn parse_against(
        raw: &str,
        flock: &FlockNames,
        source: &Value,
        target: &Value,
    ) -> Result<Address, CopyRefusal> {
        let written = written(raw, flock)?;
        let dogs = dog_names(flock, source, target);
        Ok(Address {
            sheep: written.sheep.clone(),
            path: path_of(&written, &dogs, source, target)?,
        })
    }

    #[test]
    fn a_field_a_dog_table_and_a_dog_key_each_parse() {
        let cases = [
            (
                "web.restart_delay",
                ConfigPath::Field {
                    name: "restart_delay".into(),
                },
            ),
            (
                "web.dogs.kelpie",
                ConfigPath::Dog {
                    dog: "kelpie".into(),
                    keys: vec![],
                },
            ),
            (
                "web.dogs.kelpie.rulings",
                ConfigPath::Dog {
                    dog: "kelpie".into(),
                    keys: vec!["rulings".into()],
                },
            ),
            (
                "web.dogs.kelpie.rulings.panel",
                ConfigPath::Dog {
                    dog: "kelpie".into(),
                    keys: vec!["rulings".into(), "panel".into()],
                },
            ),
        ];
        for (raw, expected) in cases {
            let address = parse(raw, &flock()).unwrap_or_else(|err| panic!("{raw}: {err}"));
            assert_eq!(address.sheep, "web");
            assert_eq!(address.path, expected, "{raw}");
            assert_eq!(address.path.dotted(), raw["web.".len()..], "{raw}");
        }
    }

    /// `we.b` is in the flock, so the split lands after it and not after `we`.
    #[test]
    fn a_sheep_name_with_a_dot_resolves_against_the_flock() {
        let address = parse("we.b.dogs.kelpie", &flock()).unwrap();
        assert_eq!(address.sheep, "we.b");
        assert_eq!(
            address.path,
            ConfigPath::Dog {
                dog: "kelpie".into(),
                keys: vec![]
            }
        );
    }

    /// `web.watch` is a sheep, so `watch` is not read as the field of `web`.
    #[test]
    fn the_longest_flock_name_wins_over_a_field_name() {
        let address = parse("web.watch.restart_delay", &flock()).unwrap();
        assert_eq!(address.sheep, "web.watch");
        assert_eq!(
            address.path,
            ConfigPath::Field {
                name: "restart_delay".into()
            }
        );
    }

    /// A name the flock has never heard of still splits, on the first segment
    /// that is a path root, so the refusal names the whole sheep name rather
    /// than the first token of it.
    #[test]
    fn an_unknown_sheep_is_named_whole() {
        let err = parse("we.c.dogs.kelpie", &flock()).unwrap_err();
        assert_eq!(
            err,
            CopyRefusal::UnknownSheep(
                "no sheep named `we.c`; `shep flock` lists the flock".to_string()
            )
        );
    }

    #[test]
    fn a_dog_name_is_refused_as_a_dog() {
        let err = parse("metrics.dogs.kelpie", &flock()).unwrap_err();
        assert_eq!(
            err,
            CopyRefusal::UnknownSheep(
                "no sheep named `metrics.dogs.kelpie`: `metrics` is a dog, and a copy writes a \
                 sheep's config"
                    .to_string()
            )
        );
    }

    #[test]
    fn an_address_with_no_path_names_the_remedies() {
        let err = parse("web", &flock()).unwrap_err();
        let CopyRefusal::Usage(message) = err else {
            panic!("expected a usage refusal, got {err:?}");
        };
        assert!(message.contains("web.restart_delay"), "{message}");
        assert!(message.contains("web.dogs.<dog>"), "{message}");
    }

    #[test]
    fn an_empty_part_is_refused() {
        assert!(matches!(
            parse("web.dogs..kelpie", &flock()),
            Err(CopyRefusal::Usage(_))
        ));
    }

    /// The three keys the shepherd refuses, each with the verb that does it.
    #[test]
    fn env_name_instances_and_the_whole_dogs_map_each_say_why() {
        let env = parse("web.env", &flock()).unwrap_err();
        assert!(env.message().contains("shep import env"), "{env}");
        assert!(env.message().contains("shep secret"), "{env}");

        let instances = parse("web.instances", &flock()).unwrap_err();
        assert!(
            instances.message().contains("shep stock web"),
            "{instances}"
        );

        let name = parse("web.name", &flock()).unwrap_err();
        assert!(name.message().contains("different sheep"), "{name}");

        let dogs = parse("web.dogs", &flock()).unwrap_err();
        assert!(dogs.message().contains("web.dogs.<dog>"), "{dogs}");
    }

    #[test]
    fn a_field_the_config_type_has_no_such_field_is_refused_by_name() {
        let err = parse("web.watchdog", &flock()).unwrap_err();
        assert_eq!(
            err,
            CopyRefusal::Usage(
                "`watchdog` is not a field a sheep's config has; `shep describe web` shows the \
                 config a copy reads"
                    .to_string()
            )
        );
    }

    #[test]
    fn a_path_that_is_neither_a_field_nor_a_dog_is_refused() {
        let err = parse("web.restart_delay.extra", &flock()).unwrap_err();
        assert_eq!(
            err,
            CopyRefusal::Usage(
                "a path is one field, `dogs.<dog>`, or `dogs.<dog>.<key>`; `web.restart_delay.extra` \
                 is none of those"
                    .to_string()
            )
        );
    }

    /// A dog key may run as deep as the dog's own
    /// table does.
    #[test]
    fn a_dog_key_may_run_deep() {
        let address = parse("web.dogs.kelpie.rulings.panel.extra", &flock()).unwrap();
        assert_eq!(
            address.path,
            ConfigPath::Dog {
                dog: "kelpie".into(),
                keys: vec!["rulings".into(), "panel".into(), "extra".into()],
            }
        );
    }

    /// A dog named `foo.bar` is one name, and the flock's list is not what
    /// says so: the table is.
    #[test]
    fn a_dog_name_with_a_dot_resolves_against_the_tables() {
        let source = json!({ "dogs": { "foo.bar": { "enabled": true } } });
        let target = json!({ "dogs": { "foo.bar": {} } });
        let whole = parse_against("web.dogs.foo.bar", &flock(), &source, &target).unwrap();
        assert_eq!(
            whole.path,
            ConfigPath::Dog {
                dog: "foo.bar".into(),
                keys: vec![]
            }
        );
        let one = parse_against("web.dogs.foo.bar.enabled", &flock(), &source, &target).unwrap();
        assert_eq!(
            one.path,
            ConfigPath::Dog {
                dog: "foo.bar".into(),
                keys: vec!["enabled".into()]
            }
        );
        assert_eq!(
            one.path.dotted(),
            "dogs.foo.bar.enabled",
            "it prints back as it was typed"
        );
    }

    /// Nothing holds `foo.bar`, so the same address is the dog `foo` and the
    /// key `bar`.
    #[test]
    fn a_dotted_dog_name_falls_back_to_a_key_when_no_table_holds_it() {
        let source = json!({ "dogs": { "foo": { "bar": { "enabled": true } } } });
        let address = parse_against("web.dogs.foo.bar", &flock(), &source, &Value::Null).unwrap();
        assert_eq!(
            address.path,
            ConfigPath::Dog {
                dog: "foo".into(),
                keys: vec!["bar".into()]
            }
        );
    }

    /// A key holding a dot is one key when a table holds it, and the two-key
    /// reading is what an address takes when it does not.
    #[test]
    fn a_key_with_a_dot_resolves_against_the_table() {
        let source = json!({ "dogs": { "kelpie": { "api.token": "abc", "api": { "limit": 5 } } } });
        let dotted =
            parse_against("web.dogs.kelpie.api.token", &flock(), &source, &Value::Null).unwrap();
        assert_eq!(
            dotted.path,
            ConfigPath::Dog {
                dog: "kelpie".into(),
                keys: vec!["api.token".into()]
            },
            "the longest key the table actually holds wins"
        );
        let nested =
            parse_against("web.dogs.kelpie.api.limit", &flock(), &source, &Value::Null).unwrap();
        assert_eq!(
            nested.path,
            ConfigPath::Dog {
                dog: "kelpie".into(),
                keys: vec!["api".into(), "limit".into()]
            }
        );
    }

    /// A dog table on the target alone is enough to make the name one name:
    /// the copy is between two sheep, and both sides are read.
    #[test]
    fn a_table_on_either_side_makes_the_name() {
        let target = json!({ "dogs": { "foo.bar": { "enabled": false } } });
        let address = parse_against("api.dogs.foo.bar", &flock(), &Value::Null, &target).unwrap();
        assert_eq!(
            address.path,
            ConfigPath::Dog {
                dog: "foo.bar".into(),
                keys: vec![]
            }
        );
    }
}
