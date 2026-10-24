//! `AppOverrides::template`: what a field held before the operator spoke for
//! it, and the stores that never recorded one.

use serde_json::json;

use super::*;

/// fails if a record written before `template` existed stops reading, or if an
/// empty template starts appearing in the file. An older shep reads this file
/// too, and an extra key in every record is what it would have to ignore.
#[test]
fn a_record_without_a_template_reads_and_writes_without_one() {
    let older = r#"{"fields":{"max_restarts":9},"declared":["name"],"declared_env":[]}"#;
    let record: AppOverrides = serde_json::from_str(older).unwrap();
    assert!(record.template.is_empty());
    let written = serde_json::to_value(&record).unwrap();
    assert!(written.get("template").is_none(), "{written}");
}

/// fails if the template does not survive the store, which is the only place
/// a reset can read it from after a shepherd restart.
#[test]
fn the_template_round_trips_through_the_store() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("overrides.json");
    let record = AppOverrides {
        fields: serde_json::Map::from_iter([("max_restarts".to_string(), json!(9))]),
        template: serde_json::Map::from_iter([("max_restarts".to_string(), json!(4))]),
        ..AppOverrides::default()
    };
    put(&path, "web", &record).unwrap();
    assert_eq!(get(&path, "web").unwrap(), Some(record));
}

/// fails if `Debug` prints a template's values. They are the Flockfile's
/// values for fields like `args` and `cwd`, which routinely hold a token.
#[test]
fn debug_counts_the_template_and_prints_none_of_it() {
    let record = AppOverrides {
        template: serde_json::Map::from_iter([("args".to_string(), json!(["--token=hunter2"]))]),
        ..AppOverrides::default()
    };
    let rendered = format!("{record:?}");
    assert!(!rendered.contains("hunter2"), "leaked: {rendered}");
    assert!(rendered.contains("template: <1 fields>"), "{rendered}");
}
