//! What an `unset` proposes to the dog it asks: the table as it would be left,
//! not the table as it stands.
//!
//! A dog is asked about the state the write ends in, so a removal has to carry
//! what survives it, and a table removed whole has to carry nothing.

use serde_json::{Map, json};

use super::Write;
use super::plan::plan_dog_table as set_plan_dog_table;
use super::unset::{plan_dog_table, plan_section};

/// A sheep's config with one dog table, spelled as a Flockfile spells it.
fn config() -> serde_json::Value {
    json!({
        "name": "api",
        "dogs": {
            "kelpie": {
                "git": { "checkout": "/srv/api", "merging": "ff" },
                "options": {}
            }
        }
    })
}

fn keys(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|key| (*key).to_string()).collect()
}

/// The key is gone from what the dog is shown, and its neighbour is there.
#[test]
fn a_removal_proposes_the_table_that_survives_it() {
    let plan = plan_dog_table("api", "kelpie", &keys(&["git", "merging"]), &config()).unwrap();

    assert!(matches!(plan.write, Write::SheepDogTable { .. }));
    assert_eq!(
        serde_json::to_value(&plan.table).unwrap(),
        json!({ "git": { "checkout": "/srv/api" }, "options": {} })
    );
}

/// The whole table goes, so the dog is asked about a holder that ends up
/// holding nothing, which is the dog's own defaults.
#[test]
fn a_table_removed_whole_proposes_no_table() {
    let plan = plan_dog_table("api", "kelpie", &keys(&[]), &config()).unwrap();

    assert!(matches!(plan.write, Write::DropSheepDogTable { .. }));
    assert_eq!(
        plan.table,
        Map::new(),
        "the table is gone, and proposing it as still there would ask the wrong question"
    );
}

/// A dog's own section is proposed as the table the shepherd parses out of the
/// text that is left, comment and all.
#[test]
fn a_section_removal_proposes_the_section_that_is_left() {
    let doc =
        "repo = \"/srv/project\"\n\n[worker]\nallowed_domains = [\"github.com\"]\ntimeout = 30\n"
            .parse::<toml_edit::DocumentMut>()
            .unwrap();

    let plan = plan_section(doc, "kelpie", &keys(&["worker", "timeout"])).unwrap();

    assert!(matches!(plan.write, Write::DogSection { .. }));
    assert_eq!(
        serde_json::to_value(&plan.table).unwrap(),
        json!({
            "repo": "/srv/project",
            "worker": { "allowed_domains": ["github.com"] },
        })
    );
}

/// The two halves of one address agree: what `shep set` proposes and what
/// `shep unset` proposes are the same holder, read before and after.
#[test]
fn set_and_unset_propose_the_same_holder() {
    let written = set_plan_dog_table(
        "api",
        "kelpie",
        &keys(&["git", "merging"]),
        &config(),
        &json!("rebase"),
    )
    .unwrap();
    let removed = plan_dog_table("api", "kelpie", &keys(&["git", "merging"]), &config()).unwrap();

    assert_eq!(
        serde_json::to_value(&written.table).unwrap(),
        json!({
            "git": { "checkout": "/srv/api", "merging": "rebase" },
            "options": {},
        })
    );
    assert_eq!(
        serde_json::to_value(&removed.table).unwrap(),
        json!({ "git": { "checkout": "/srv/api" }, "options": {} })
    );
}
