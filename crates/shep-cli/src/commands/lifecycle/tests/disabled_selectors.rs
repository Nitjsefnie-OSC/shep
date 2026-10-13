//! `flock_matches` and a disabled sheep: a wildcard passes it by and a name
//! still reaches it.

use shep_core::protocol::ProcessInfo;
use shep_core::status::ProcStatus;

use super::*;

/// `api` and `site` in fold `web` (`api` disabled), plus a disabled `worker`
/// and an enabled `cron`, none in a fold.
fn flock_with_two_disabled() -> Vec<ProcessInfo> {
    let stopped = |id, name: &str, fold: Option<&str>, disabled: bool| {
        ProcessInfo::builder(id, name, ProcStatus::Stopped)
            .fold(fold.map(str::to_string))
            .disabled(disabled.then_some(true))
            .build()
    };
    vec![
        stopped(0, "api", Some("web"), true),
        stopped(1, "site", Some("web"), false),
        stopped(2, "worker", None, true),
        stopped(3, "cron", None, false),
    ]
}

fn names_for(target: &str) -> Vec<String> {
    let selector = ProcessSelector::parse(target).expect("the fixture uses valid selectors");
    flock_matches(&selector, &flock_with_two_disabled())
        .into_iter()
        .map(|info| info.name)
        .collect()
}

/// Fails if the wildcard branch stops checking the mark: `start all` would
/// name `api` and `worker`, and the shepherd would refuse them.
#[test]
fn a_wildcard_passes_a_disabled_sheep_by() {
    assert_eq!(names_for("all"), vec!["site", "cron"]);
    assert_eq!(names_for("fold:web"), vec!["site"]);
    assert_eq!(names_for("/^w/"), Vec::<String>::new());
}

#[test]
fn a_bare_fold_name_passes_a_disabled_sheep_by() {
    assert_eq!(names_for("web"), vec!["site"]);
}

#[test]
fn naming_a_disabled_sheep_still_reaches_it() {
    assert_eq!(names_for("api"), vec!["api"]);
    assert_eq!(names_for("2"), vec!["worker"]);
}

#[test]
fn an_all_that_found_only_disabled_sheep_says_to_enable_them() {
    let flock: Vec<ProcessInfo> = flock_with_two_disabled()
        .into_iter()
        .filter(|info| info.disabled == Some(true))
        .collect();

    let miss = selector_miss("all", &ProcessSelector::All, &flock).expect("a miss message");

    assert!(miss.contains("`shep enable all`"), "{miss}");
}
