//! Which dogs a provider's push can reach: the ones whose `dogs.toml`
//! section names a value in the pushed namespace.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use shep_core::config::DogsConfig;

use super::config::dog_section;
use super::secrets::section_references;

/// Every dog whose section in `dogs_config` names `namespace`, with the
/// keys it names there.
///
/// A reference is read through [`section_references`], so a string that
/// [`super::resolve_section`] would not substitute names nothing. An
/// un-namespaced `{{secret:KEY}}` is the operator's own store, not a
/// provider's, and is not counted. A missing, unreadable or invalid file
/// names nobody, and so does a section that does not parse: the push that
/// asked is not the place to report either.
#[must_use]
pub fn dogs_referencing(dogs_config: &Path, namespace: &str) -> BTreeMap<String, BTreeSet<String>> {
    let Ok(source) = std::fs::read_to_string(dogs_config) else {
        return BTreeMap::new();
    };
    let Ok(config) = DogsConfig::load(Some(&source)) else {
        return BTreeMap::new();
    };
    let prefix = format!("{namespace}/");
    config
        .dog
        .keys()
        .filter_map(|dog| {
            let section = dog_section(dogs_config, dog).ok()?;
            let keys: BTreeSet<String> = section_references(&section)
                .into_iter()
                .filter_map(|reference| reference.strip_prefix(&prefix).map(str::to_owned))
                .collect();
            (!keys.is_empty()).then(|| (dog.clone(), keys))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(dogs_toml: &str, namespace: &str) -> BTreeMap<String, BTreeSet<String>> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dogs.toml");
        std::fs::write(&path, dogs_toml).unwrap();
        dogs_referencing(&path, namespace)
    }

    fn keys(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn each_dog_comes_back_with_the_keys_it_names_in_that_namespace() {
        let found = named(
            "[bark]\nurl = \"{{secret:vercel/URL}}\"\ntoken = \"{{secret:vercel/TOKEN}} {{secret:other/X}}\"\n\
             [metrics.auth]\nkey = \"{{secret:vercel/TOKEN}}\"\n\
             [quiet]\npoll = \"5s\"\n\
             [own]\nk = \"{{secret:TOKEN}}\"\n",
            "vercel",
        );
        assert_eq!(
            found,
            BTreeMap::from([
                ("bark".to_string(), keys(&["TOKEN", "URL"])),
                ("metrics".to_string(), keys(&["TOKEN"])),
            ])
        );
    }

    #[test]
    fn a_namespace_nobody_names_finds_nobody() {
        assert!(named("[bark]\nt = \"{{secret:vercel/T}}\"\n", "other").is_empty());
    }

    #[test]
    fn a_missing_or_invalid_file_names_nobody() {
        let dir = tempfile::tempdir().unwrap();
        assert!(dogs_referencing(&dir.path().join("dogs.toml"), "vercel").is_empty());
        assert!(named("[bark\nt = ", "vercel").is_empty());
    }
}
