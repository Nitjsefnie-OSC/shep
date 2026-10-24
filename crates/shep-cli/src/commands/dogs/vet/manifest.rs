//! Reading a dog's manifest for `shep adopt` and `shep enable`: asked of the
//! binary with [`MANIFEST_FLAG`], and cached by the binary's sha256 so a
//! build is asked once.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use shep_core::dogs::{DogVersion, MANIFEST_FLAG, manifest::Manifest};

use super::{AdoptRefusal, DogSchema, ask};

/// The directory under `$SHEP_HOME` holding one `<sha256>.json` per binary
/// that answered [`MANIFEST_FLAG`].
const CACHE_DIR: &str = "manifests";

/// What `path` declares, from the cache when this exact binary has been read
/// before and from the binary otherwise.
///
/// `Ok(None)` is a dog with no manifest: a non-zero exit, no output, output
/// that is not a manifest, or a run that outlasted `budget`. That is never a
/// fault, and is never cached, so a dog that gains a manifest is noticed on
/// the next call. The cache is derived and safe to delete: one that cannot
/// be read, parsed or written is skipped.
///
/// # Errors
/// [`AdoptRefusal::WillNotExec`], and only that, from the probe.
pub(crate) fn read_manifest(
    path: &Path,
    home: &Path,
    name: &str,
    budget: Duration,
) -> Result<Option<Manifest>, AdoptRefusal> {
    let cached = sha256_hex(path).map(|digest| home.join(CACHE_DIR).join(format!("{digest}.json")));
    if let Some(manifest) = cached
        .as_deref()
        .and_then(|file| std::fs::read_to_string(file).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        return Ok(Some(manifest));
    }
    let Some(text) = ask(path, MANIFEST_FLAG, home, name, budget)? else {
        return Ok(None);
    };
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        return Ok(None);
    };
    if let Some(file) = &cached {
        let _ = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(file, &text));
    }
    Ok(Some(manifest))
}

/// `manifest` as the `--version` answer it replaces.
pub(super) fn version_of(manifest: &Manifest) -> DogVersion {
    DogVersion {
        version: manifest.version.clone(),
        protocol: Some(manifest.protocol),
        channel: manifest.channel,
    }
}

/// `manifest`'s settings schema as the `--schema` answer it replaces.
pub(super) fn schema_of(manifest: &Manifest) -> DogSchema {
    manifest
        .settings
        .clone()
        .map_or(DogSchema::Silent, DogSchema::Published)
}

/// The lowercase hex sha256 of the file at `path`, or `None` when it cannot
/// be read.
fn sha256_hex(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        match file.read(&mut chunk).ok()? {
            0 => break,
            read => context.update(&chunk[..read]),
        }
    }
    Some(
        context
            .finish()
            .as_ref()
            .iter()
            .fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02x}");
                hex
            }),
    )
}

#[cfg(all(test, unix))]
mod tests;
