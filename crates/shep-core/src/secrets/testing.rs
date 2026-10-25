//! Fixtures and helpers shared by this module's tests.

use super::provider::ProviderCache;
use std::collections::{BTreeMap, BTreeSet};

/// A cache holding `vercel/PRESENT` for `environment`, pushed as that one
/// pair and no other.
pub(super) fn vercel_env(environment: &str) -> ProviderCache {
    ProviderCache::new(
        BTreeMap::from([(
            "vercel".to_string(),
            BTreeMap::from([(
                "PRESENT".to_string(),
                BTreeMap::from([(environment.to_string(), "v".to_string())]),
            )]),
        )]),
        BTreeMap::from([(
            "vercel".to_string(),
            BTreeSet::from([environment.to_string()]),
        )]),
    )
}
