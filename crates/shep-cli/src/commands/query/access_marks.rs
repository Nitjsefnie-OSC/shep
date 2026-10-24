//! The marks on a dog's access, read from the operator's `shep.toml` for a
//! listing of this machine's flock.
//!
//! The shepherd judges `granted` alone, so what waits and what the operator
//! has not reviewed lives only in the CLI's record.

use shep_core::paths::ShepPaths;
use shep_core::protocol::{DogAccess, ProcessInfo};

use crate::commands::shep_toml::ShepToml;

/// Puts [`DogAccess`] on every dog row of `procs`, leaving sheep alone.
///
/// A `shep.toml` that cannot be read marks nothing: a listing never fails
/// over a record only `shep enable` and `shep dogs grant` need to be right
/// about.
pub(crate) fn mark_access(procs: &mut [ProcessInfo], paths: &ShepPaths) {
    let Ok(cfg) = ShepToml::read_only(&paths.daemon_config) else {
        return;
    };
    for proc in procs.iter_mut().filter(|proc| proc.dog.is_some()) {
        // A disabled dog runs nothing, so it has no full access to flag.
        let undeclared = proc.disabled != Some(true)
            && proc.dog.as_ref().is_some_and(|dog| dog.grants().is_none());
        let record = cfg.grant_record(&proc.name).ok().flatten();
        proc.access = Some(DogAccess {
            undeclared,
            not_reviewed: record.as_ref().is_some_and(|record| !record.reviewed),
            waiting: record
                .map(|record| record.waiting.names().map(str::to_owned).collect())
                .unwrap_or_default(),
        });
    }
}
