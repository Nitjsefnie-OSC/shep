//! What a dog's manifest says it needs, checked when `shep adopt` or `shep
//! enable` reads it. Warnings only: a dog whose needs are unmet is still
//! adopted and enabled, and installing a missing one is a later step.

use shep_core::dogs::manifest::Manifest;

use crate::output::Streams;

/// [`emit_notice`](crate::output::emit_notice) code for a dependency dog that
/// is not adopted; not a failure.
const DOG_DEPENDENCY_NOTICE: &str = "dog_dependency_missing";

/// [`emit_notice`](crate::output::emit_notice) code for a dog that needs a
/// newer shep than this one; not a failure.
const DOG_SHEP_TOO_OLD_NOTICE: &str = "dog_needs_newer_shep";

/// Warns of each dog `manifest` depends on that is neither built in nor in
/// `adopted`, and of a minimum shep newer than this build.
///
/// A dependency is checked by name: which version of it is installed is not
/// read. A minimum that is not a version is not judged.
pub(super) fn warn_of_unmet_depends(
    streams: &mut Streams<'_>,
    name: &str,
    manifest: &Manifest,
    adopted: &[String],
) {
    for dependency in &manifest.depends.dogs {
        let present = crate::dog::BUILT_IN_DOGS.contains(&dependency.name.as_str())
            || adopted.contains(&dependency.name);
        if !present {
            let message = format!(
                "`{name}` depends on `{dep}`, which is not adopted, so `{name}` may not work \
                 until you run `shep adopt` for `{dep}`",
                dep = dependency.name
            );
            streams.aside(DOG_DEPENDENCY_NOTICE, &message);
        }
    }
    let Some(minimum) = &manifest.depends.shep else {
        return;
    };
    let this = env!("CARGO_PKG_VERSION");
    let too_old = match (
        semver::Version::parse(minimum),
        semver::Version::parse(this),
    ) {
        (Ok(minimum), Ok(this)) => this < minimum,
        _ => false,
    };
    if too_old {
        let message = format!(
            "`{name}` needs shep {minimum} or newer, and this is shep {this}, so `{name}` may \
             not work until you upgrade shep"
        );
        streams.aside(DOG_SHEP_TOO_OLD_NOTICE, &message);
    }
}
