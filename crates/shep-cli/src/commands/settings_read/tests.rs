use serde_json::json;

use super::*;

/// The flock every test here resolves against: two sheep, one of them
/// named with a dot, and two dogs.
fn flock() -> FlockNames {
    FlockNames {
        sheep: ["api", "we.b", "web"]
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        dogs: ["metrics", "kelpie"]
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    }
}
/// A sheep's config as `Request::SheepConfig` answers it: `env` emptied,
/// every unset `Option` field `null`.
fn config() -> Json {
    json!({
        "name": "api",
        "script": "./api.js",
        "args": ["--port", "8080"],
        "env": {},
        "cwd": null,
        "instances": 2,
        "autorestart": true,
        "max_memory": "1G",
        "restart_delay": null,
        "watch_paths": ["src/**/*.js"],
        "probe": {"type": "http", "url": "http://127.0.0.1:8080/healthz"},
        "dogs": {
            "kelpie": {
                "enabled": true,
                "git": {"checkout": "/srv/api", "merging": "ff"},
                "api.token": "literal-dotted-key"
            },
            "metrics": {"interval": 10}
        }
    })
}
/// The written address `raw` names, resolved against [`flock`]'s sheep.
fn sheep_written(raw: &str) -> Written {
    let segments = segments(raw).unwrap_or_else(|err| panic!("{raw}: {err}"));
    let name = address::longest_in(&segments, &flock().sheep)
        .unwrap_or_else(|| panic!("{raw}: no sheep in this flock is named by it"));
    let depth = name.split('.').count();
    Written {
        holder: Holder::Sheep,
        name,
        rest: segments[depth..].to_vec(),
    }
}
/// The written address a dog read lands on once [`dog_candidates`] has
/// found `name` and the segments left over are its path.
fn dog_written(name: &str, path: &[&str]) -> Written {
    Written {
        holder: Holder::Dog,
        name: name.to_string(),
        rest: path.iter().map(|part| (*part).to_string()).collect(),
    }
}
/// The TOML `shep get {raw}` prints for [`config`]'s sheep.
fn sheep_toml(raw: &str) -> String {
    let written = sheep_written(raw);
    let dogs = address::DogNames::new(&flock(), &config(), &Json::Null);
    let steps = sheep_steps(&written.name, &written.rest, &dogs, &config())
        .unwrap_or_else(|err| panic!("{raw}: {err}"));
    sheep_document(&written, &config(), &steps).unwrap_or_else(|err| panic!("{raw}: {err}"))
}
/// `shep get api` prints the config as a Flockfile spells it: the scalars
/// first, every dog table under its `dogs.` header, and no `env` header
/// for a table the shepherd empties before it answers.
/// The config arrives as a JSON map, which this crate's `serde_json` has
/// in key order rather than Flockfile order; the tables underneath keep
/// the headers a Flockfile spells, and `[dogs]` gets none of its own.
#[test]
fn a_sheep_name_alone_prints_its_config() {
    let toml = sheep_toml("api");
    assert_eq!(
        toml,
        "args = [\"--port\", \"8080\"]\n\
         autorestart = true\n\
         instances = 2\n\
         max_memory = \"1G\"\n\
         name = \"api\"\n\
         script = \"./api.js\"\n\
         watch_paths = [\"src/**/*.js\"]\n\
         \n\
         [dogs.kelpie]\n\"api.token\" = \"literal-dotted-key\"\nenabled = true\n\
         \n\
         [dogs.kelpie.git]\ncheckout = \"/srv/api\"\nmerging = \"ff\"\n\
         \n\
         [dogs.metrics]\ninterval = 10\n\
         \n\
         [probe]\ntype = \"http\"\nurl = \"http://127.0.0.1:8080/healthz\"\n"
    );
}
/// An unset field prints nothing: there is no value to print, and a
/// `null` is not TOML.
#[test]
fn an_unset_field_is_left_out_of_a_whole_config() {
    let toml = sheep_toml("api");
    assert!(!toml.contains("cwd"), "{toml}");
    assert!(!toml.contains("restart_delay"), "{toml}");
    assert!(!toml.contains("[env]"), "{toml}");
}
#[test]
fn one_field_prints_one_line() {
    assert_eq!(sheep_toml("api.max_memory"), "max_memory = \"1G\"\n");
}
/// The header is the one the Flockfile spells, so the answer pastes back
/// into the file it came from.
#[test]
fn a_dogs_table_inside_a_sheep_prints_its_own_header() {
    assert_eq!(
        sheep_toml("api.dogs.kelpie.git"),
        "[dogs.kelpie.git]\ncheckout = \"/srv/api\"\nmerging = \"ff\"\n"
    );
}
#[test]
fn every_dog_table_prints_when_no_dog_is_named() {
    let toml = sheep_toml("api.dogs");
    assert!(toml.contains("[dogs.kelpie]"), "{toml}");
    assert!(toml.contains("[dogs.metrics]"), "{toml}");
}
/// An empty table the Flockfile wrote a header for is a setting the operator
/// set, and the header is the only way to say it is there: a dog's settings
/// are opaque to shep, so an empty table cannot be told apart from a missing
/// one. Dropping it would lose configuration from a paste-back answer.
#[test]
fn an_empty_dog_table_keeps_its_header() {
    let mut cfg = config();
    cfg["dogs"]["kelpie"]["options"] = json!({});
    let written = sheep_written("api.dogs.kelpie.options");
    let toml = sheep_document(&written, &cfg, &written.rest).unwrap();
    assert_eq!(toml, "[dogs.kelpie.options]\n", "{toml}");

    let whole = sheep_document(&sheep_written("api"), &cfg, &[]).unwrap();
    assert!(whole.contains("[dogs.kelpie.options]"), "{whole}");
}
/// The sheep's own `dogs` container is the one table of a config that prints
/// no header of its own: a Flockfile spells `[app.dogs.kelpie]` and never
/// `[app.dogs]`, and the shepherd answers an empty `dogs` for a sheep with no
/// per-sheep dog settings, which is the same config as one that says nothing
/// about dogs.
#[test]
fn a_sheep_with_no_dogs_prints_no_dogs_header() {
    let mut cfg = config();
    cfg["dogs"] = json!({});
    let whole = sheep_document(&sheep_written("api"), &cfg, &[]).unwrap();
    assert!(!whole.contains("dogs"), "{whole}");

    let bare = sheep_written("api.dogs");
    assert_eq!(
        sheep_document(&bare, &cfg, &bare.rest).unwrap(),
        "",
        "an empty container has nothing to spell"
    );
}
/// A key inside a dog's table prints beside the table it sits in, which
/// is where a reader would go to change it.
#[test]
fn one_key_of_a_dog_table_prints_under_that_table() {
    assert_eq!(
        sheep_toml("api.dogs.kelpie.git.merging"),
        "[dogs.kelpie.git]\nmerging = \"ff\"\n"
    );
}
/// A key that itself carries a dot is one key when the table holds it as
/// one, and it prints quoted so reading it back gives the same key.
#[test]
fn a_dotted_key_of_a_dog_table_stays_one_key() {
    assert_eq!(
        sheep_toml("api.dogs.kelpie.api.token"),
        "[dogs.kelpie]\n\"api.token\" = \"literal-dotted-key\"\n"
    );
}
/// The sheep half of the rule `shep copy`'s resolver carries: a dotted key
/// inside a dotted key is two keys, and the read stays inside the table the
/// first one names rather than falling out of it.
#[test]
fn a_dotted_key_inside_a_dotted_key_reads_from_a_sheeps_dog_table() {
    let mut cfg = config();
    cfg["dogs"]["kelpie"]["api.token"] = json!({ "auth.key": 42 });
    let dogs = address::DogNames::new(&flock(), &cfg, &Json::Null);
    let written = sheep_written("api.dogs.kelpie.api.token.auth.key");
    let steps = sheep_steps(&written.name, &written.rest, &dogs, &cfg).unwrap();
    assert_eq!(
        steps,
        ["dogs", "kelpie", "api.token", "auth.key"],
        "{steps:?}"
    );
    assert_eq!(
        sheep_document(&written, &cfg, &steps).unwrap(),
        "[dogs.kelpie.\"api.token\"]\n\"auth.key\" = 42\n"
    );
}
/// `we.b` is a sheep, so the split lands after it rather than after `we`.
#[test]
fn a_sheep_name_with_a_dot_is_one_name() {
    let written = sheep_written("we.b.dogs.kelpie");
    assert_eq!(written.holder, Holder::Sheep);
    assert_eq!(written.name, "we.b");
    assert_eq!(written.rest, ["dogs", "kelpie"]);
}
/// `dogs.toml` spells a dog's sub-table as `[kelpie.worker]`, a header
/// path rather than a name, so the dog is the longest candidate the
/// shepherd answers a section for.
#[test]
fn a_dog_address_offers_its_names_longest_first() {
    assert_eq!(
        dog_candidates(&segments("kelpie.worker.token").unwrap()),
        ["kelpie.worker.token", "kelpie.worker", "kelpie"]
    );
    assert_eq!(dog_candidates(&segments("kelpie").unwrap()), ["kelpie"]);
}
#[test]
fn a_name_neither_a_sheep_nor_a_dog_names_the_store() {
    let refusal = ReadRefusal::Unknown(no_such_name(&segments("bark.cooldown").unwrap()));
    assert_eq!(refusal.code(), ExitCode::NotFound);
    assert!(refusal.message().contains("shep kv get"), "{refusal}");
    assert!(refusal.message().contains("shep dogs"), "{refusal}");
}
/// A dog the flock runs and `dogs.toml` says nothing about is not a wrong
/// name, and the sentence says what is actually true of it: the dog named
/// is the one the flock carries, and `shep enable` is aimed at that name.
#[test]
fn a_dog_with_no_section_says_its_settings_are_defaults() {
    let refusal = ReadRefusal::Empty(all_defaults("metrics", &segments("metrics").unwrap()));
    assert_eq!(refusal.code(), ExitCode::NotFound);
    assert!(
        refusal.message().starts_with("`metrics` is a dog"),
        "{refusal}"
    );
    assert!(refusal.message().contains("default"), "{refusal}");
    assert!(
        refusal.message().contains("shep enable metrics"),
        "{refusal}"
    );
}
/// fails if the address is reported as the settings holder: `bind` is a
/// path inside `metrics`, and the operator has to be able to tell which
/// name to hand `shep enable`.
#[test]
fn a_path_inside_a_default_only_dog_names_the_dog_and_not_the_path() {
    let refusal = ReadRefusal::Empty(all_defaults("metrics", &segments("metrics.bind").unwrap()));
    assert!(
        refusal.message().starts_with("`metrics` is a dog"),
        "{refusal}"
    );
    assert!(
        refusal
            .message()
            .contains("`metrics.bind` names nothing inside it"),
        "{refusal}"
    );
    assert!(
        refusal.message().contains("shep enable metrics"),
        "{refusal}"
    );
}
/// A dog whose own name is dotted is that whole name: `shep enable` takes
/// it whole, and a suggestion built from the first segment names a dog no
/// flock has.
#[test]
fn a_dotted_dog_name_stays_one_name() {
    let refusal = ReadRefusal::Empty(all_defaults(
        "kelpie.worker",
        &segments("kelpie.worker.token").unwrap(),
    ));
    assert!(
        refusal.message().starts_with("`kelpie.worker` is a dog"),
        "{refusal}"
    );
    assert!(
        refusal.message().contains("shep enable kelpie.worker`"),
        "{refusal}"
    );
    assert!(
        !refusal.message().contains("shep enable kelpie `"),
        "{refusal}"
    );
}
#[test]
fn an_empty_part_of_an_address_is_a_usage_error() {
    assert!(matches!(
        segments("api.dogs..kelpie"),
        Err(ReadRefusal::Usage(_))
    ));
}
#[test]
fn a_field_no_sheep_has_is_refused_by_name() {
    let written = sheep_written("api.watchdog");
    let dogs = address::DogNames::new(&flock(), &config(), &Json::Null);
    let Err(refusal) = sheep_steps(&written.name, &written.rest, &dogs, &config()) else {
        panic!("`watchdog` is no field of a sheep's config");
    };
    assert_eq!(refusal.code(), ExitCode::Usage);
    assert!(refusal.message().contains("`watchdog`"), "{refusal}");
}
/// `env` values never reach a client, so the refusal names the two verbs
/// that do hold them.
#[test]
fn env_names_the_verbs_that_hold_a_sheeps_env() {
    let written = sheep_written("api.env");
    let dogs = address::DogNames::new(&flock(), &config(), &Json::Null);
    let Err(refusal) = sheep_steps(&written.name, &written.rest, &dogs, &config()) else {
        panic!("`env` is a refusal, not a read");
    };
    assert_eq!(refusal.code(), ExitCode::InvalidConfig);
    assert!(refusal.message().contains("shep import env"), "{refusal}");
    assert!(
        refusal.message().contains("shep describe api"),
        "the refusal has to name the sheep it refused, {refusal}"
    );
    assert!(!refusal.message().contains("{sheep}"), "{refusal}");
}
#[test]
fn a_path_nothing_is_set_at_is_not_found() {
    let written = sheep_written("api.dogs.kelpie.git.remote");
    let dogs = address::DogNames::new(&flock(), &config(), &Json::Null);
    let steps = sheep_steps(&written.name, &written.rest, &dogs, &config()).unwrap();
    let Err(refusal) = sheep_document(&written, &config(), &steps) else {
        panic!("nothing holds `api.dogs.kelpie.git.remote`");
    };
    assert_eq!(refusal.code(), ExitCode::NotFound);
    assert!(
        refusal.message().contains("api.dogs.kelpie.git.remote"),
        "{refusal}"
    );
}
/// An `Option` field the operator never wrote is a path nothing is set at.
#[test]
fn an_unset_field_named_directly_is_not_found() {
    let written = sheep_written("api.restart_delay");
    let dogs = address::DogNames::new(&flock(), &config(), &Json::Null);
    let steps = sheep_steps(&written.name, &written.rest, &dogs, &config()).unwrap();
    assert!(matches!(
        sheep_document(&written, &config(), &steps),
        Err(ReadRefusal::Empty(_))
    ));
}
/// A dog's section can hold a key spelled with a dot in it, quoted in the
/// file: `[kelpie."api.token"]` holding `"auth.key" = 42`. The address names
/// it segment by segment, and the walk descends by the key it matched rather
/// than by the segments that spelled it: looking for `api` then `token` lands
/// outside the table, and the setting reads as absent.
#[test]
fn a_dotted_table_header_of_a_dogs_section_stays_one_key_on_the_way_down() {
    let section = "repo = \"/srv/project\"\n\n[\"api.token\"]\n\"auth.key\" = 42\n";
    let doc: DocumentMut = section.parse().unwrap();
    let written = dog_written("kelpie", &["api", "token", "auth", "key"]);
    let steps = dog_steps(doc.as_table(), &written.rest);
    assert_eq!(steps, ["api.token", "auth.key"], "{steps:?}");
    assert_eq!(
        dog_document(&written, &doc, &steps).unwrap(),
        "[kelpie.\"api.token\"]\n\"auth.key\" = 42\n"
    );
}

/// A dog's section prints under its own name, which is how `dogs.toml`
/// spells it, and the comments inside it survive.
#[test]
fn a_dogs_own_section_prints_under_its_name_and_keeps_its_comments() {
    let toml = dog_toml("kelpie", &[], SECTION);
    assert!(toml.starts_with("[kelpie]\n"), "{toml}");
    assert!(toml.contains("# what it may fetch"), "{toml}");
    assert!(toml.contains("[kelpie.worker]"), "{toml}");
}
#[test]
fn one_key_of_a_dogs_section_prints_under_the_dog() {
    let toml = dog_toml("kelpie", &["worker"], SECTION);
    assert_eq!(
        toml,
        "[kelpie.worker]\nallowed_domains = [\"github.com\"]\ntimeout = 30\n"
    );
}
/// A dotted key written `api.token = ...` inside a section is a path
/// through a table the file spells with a header, and it reads the same
/// either way.
#[test]
fn a_dotted_key_inside_a_dogs_section_reads_as_a_table() {
    let toml = dog_toml("kelpie", &["api", "token"], SECTION);
    assert_eq!(toml, "[kelpie.api]\ntoken = \"literal-dotted-key\"\n");
}
#[test]
fn a_key_no_dog_holds_is_not_found() {
    let written = dog_written("kelpie", &["worker", "nope"]);
    let doc: DocumentMut = SECTION.parse().unwrap();
    let steps = dog_steps(doc.as_table(), &written.rest);
    let Err(refusal) = dog_document(&written, &doc, &steps) else {
        panic!("nothing holds `kelpie.worker.nope`");
    };
    assert_eq!(refusal.code(), ExitCode::NotFound);
}
/// The section `shep adopt` writes for a work-item dog: a scalar, a
/// dotted key, and a sub-table with a comment of its own.
const SECTION: &str = "\
# kelpie reads the repo it is adopted into
repo = \"/srv/project\"
api.token = \"literal-dotted-key\"
# what it may fetch
[worker]
allowed_domains = [\"github.com\"]
timeout = 30
";
/// The TOML `shep get {name}[.{path}]` prints for a dog whose section is
/// `section`.
fn dog_toml(name: &str, path: &[&str], section: &str) -> String {
    let written = dog_written(name, path);
    let doc: DocumentMut = section.parse().unwrap();
    let steps = dog_steps(doc.as_table(), &written.rest);
    dog_document(&written, &doc, &steps)
        .unwrap_or_else(|err| panic!("{name}.{}: {err}", path.join(".")))
}
