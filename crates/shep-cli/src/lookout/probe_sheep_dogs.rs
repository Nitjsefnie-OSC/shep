//! Asking every dog this shepherd knows for its per-sheep schema.
//!
//! Run off the UI task when the dogs sub-screen opens. A built-in dog is
//! this binary and answers in-process; an adopted one is spawned with the
//! schema flag, all of them at once, so the wait is the slowest dog's rather
//! than the sum. Nothing here is stored.

use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value};
use shep_core::dogs::SHEEP_SCHEMA_KEY;

use super::pane::SheepDogEntry;
use crate::commands::dogs::{DogSchema, ask_schema};
use crate::commands::settings::dog_candidates;
use crate::commands::shep_toml::ShepToml;

/// One entry per dog `daemon_config` names or shep builds in, each with its
/// per-sheep schema where it publishes one. Each adopted dog has `budget`
/// to answer, `VERSION_BUDGET` outside a test.
///
/// A `shep.toml` that cannot be read lists no dog, which leaves every table
/// the sheep carries read-only: without a schema nothing is edited.
pub(super) fn probe_sheep_dogs(
    daemon_config: &Path,
    home: &Path,
    budget: Duration,
) -> Vec<SheepDogEntry> {
    let Ok(doc) = ShepToml::read_only(daemon_config) else {
        return Vec::new();
    };
    let candidates = dog_candidates(&doc);
    std::thread::scope(|scope| {
        let asks: Vec<_> = candidates
            .into_iter()
            .map(|dog| {
                scope.spawn(move || {
                    let answer = match (crate::dog::builtin_schema(&dog.name), &dog.adopted_path) {
                        (Some(schema), _) => Some(schema),
                        (None, Some(path)) => match ask_schema(path, home, &dog.name, budget) {
                            DogSchema::Published(schema) => Some(schema),
                            DogSchema::Silent | DogSchema::Unreadable => None,
                        },
                        (None, None) => None,
                    };
                    SheepDogEntry {
                        schema: answer.as_ref().and_then(sheep_schema),
                        name: dog.name,
                        adopted_path: dog.adopted_path,
                    }
                })
            })
            .collect();
        asks.into_iter().filter_map(|ask| ask.join().ok()).collect()
    })
}

/// `pub(crate)`: `shep set` validates a dog's table on a sheep against the
/// same schema the pane edits its rows from (issue #1096).
///
/// `root`'s `x-shep-sheep` value, with `root`'s own `$defs` attached so each
/// `$ref` in it resolves where it points. [`None`] when there is no such key,
/// or when it is not a schema object. A definition the sheep schema carries
/// itself wins over the root's of the same name.
pub(crate) fn sheep_schema(root: &Value) -> Option<Value> {
    let mut sheep = root.get(SHEEP_SCHEMA_KEY)?.as_object()?.clone();
    if let Some(root_defs) = root.get("$defs").and_then(Value::as_object) {
        let defs = sheep
            .entry("$defs")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(defs) = defs {
            for (name, def) in root_defs {
                defs.entry(name.clone()).or_insert_with(|| def.clone());
            }
        }
    }
    Some(Value::Object(sheep))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_sheep_schema_carries_the_roots_defs() {
        let root = json!({
            "properties": { "poll": { "type": "string" } },
            "x-shep-sheep": { "$ref": "#/$defs/ProjectSettings" },
            "$defs": { "ProjectSettings": { "type": "object" } },
        });
        assert_eq!(
            sheep_schema(&root),
            Some(json!({
                "$ref": "#/$defs/ProjectSettings",
                "$defs": { "ProjectSettings": { "type": "object" } },
            }))
        );
    }

    #[test]
    fn a_schema_without_the_key_or_with_a_bare_value_has_none() {
        assert_eq!(sheep_schema(&json!({ "properties": {} })), None);
        assert_eq!(sheep_schema(&json!({ "x-shep-sheep": true })), None);
    }

    #[test]
    fn the_sheep_schemas_own_definition_wins() {
        let root = json!({
            "x-shep-sheep": { "$defs": { "Hours": { "type": "string" } } },
            "$defs": { "Hours": { "type": "object" }, "Other": {} },
        });
        let sheep = sheep_schema(&root).expect("a sheep schema");
        assert_eq!(sheep["$defs"]["Hours"], json!({ "type": "string" }));
        assert_eq!(sheep["$defs"]["Other"], json!({}));
    }

    #[test]
    fn a_shep_toml_that_does_not_parse_lists_no_dog() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("shep.toml");
        std::fs::write(&config, "[daemon\n").unwrap();
        assert!(probe_sheep_dogs(&config, dir.path(), Duration::from_secs(1)).is_empty());
    }

    /// An adopted dog is its own binary, asked with the schema flag. The
    /// built-ins publish no sheep schema, so they list with none.
    #[cfg(unix)]
    #[test]
    fn an_adopted_dog_is_asked_and_a_built_in_answers_with_none() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("shep-jobs");
        let schema = json!({
            "properties": {},
            "x-shep-sheep": { "$ref": "#/$defs/Settings" },
            "$defs": { "Settings": { "type": "object" } },
        });
        std::fs::write(
            &script,
            format!("#!/bin/sh\n[ \"$1\" = --schema ] && echo '{schema}'\nexit 0\n"),
        )
        .unwrap();
        let quiet = dir.path().join("shep-quiet");
        std::fs::write(&quiet, "#!/bin/sh\nexit 0\n").unwrap();
        let broken = dir.path().join("shep-broken");
        std::fs::write(&broken, "#!/bin/sh\necho '{not json'\nexit 0\n").unwrap();
        for bin in [&script, &quiet, &broken] {
            std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let config = dir.path().join("shep.toml");
        std::fs::write(
            &config,
            format!(
                "[daemon.adopted_dogs]\njobs = {:?}\nquiet = {:?}\nbroken = {:?}\n",
                script.display().to_string(),
                quiet.display().to_string(),
                broken.display().to_string()
            ),
        )
        .unwrap();

        // Thirty seconds, not the real one: a first spawn of a new script
        // on a loaded machine outruns a second, and this is not the bound.
        let mut dogs = probe_sheep_dogs(&config, dir.path(), Duration::from_secs(30));
        dogs.sort_by(|a, b| a.name.cmp(&b.name));
        let jobs = dogs.iter().find(|dog| dog.name == "jobs").expect("jobs");
        assert_eq!(jobs.adopted_path.as_deref(), Some(script.as_path()));
        assert_eq!(
            jobs.schema,
            Some(json!({
                "$ref": "#/$defs/Settings",
                "$defs": { "Settings": { "type": "object" } },
            }))
        );
        let bark = dogs
            .iter()
            .find(|dog| dog.name == "bark")
            .expect("a built-in");
        assert_eq!(bark.schema, None);
        for silent in ["quiet", "broken"] {
            let dog = dogs.iter().find(|dog| dog.name == silent).expect(silent);
            assert_eq!(dog.schema, None, "{silent} answers no schema");
        }
    }
}
