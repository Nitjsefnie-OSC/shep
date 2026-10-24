//! Reading a dog's manifest for `shep adopt` and `shep enable`: asked of the
//! binary with [`MANIFEST_FLAG`], and cached by the binary's sha256 so a
//! build is asked once.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use shep_core::atomic_file;
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
/// be read, parsed or written is skipped, and a binary that changed while it
/// was probed is not cached.
///
/// # Errors
/// [`AdoptRefusal::WillNotExec`], and only that, from the probe.
pub(crate) fn read_manifest(
    path: &Path,
    home: &Path,
    name: &str,
    budget: Duration,
) -> Result<Option<Manifest>, AdoptRefusal> {
    let digest = sha256_hex(path);
    let cached = digest
        .as_ref()
        .map(|digest| home.join(CACHE_DIR).join(format!("{digest}.json")));
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
    // A binary swapped while it ran may have answered as the new build, and
    // the old digest would file that answer under the wrong key.
    if let Some(file) = cached.filter(|_| sha256_hex(path) == digest) {
        let _ = store(&file, &text);
    }
    Ok(Some(manifest))
}

/// Writes `text` to `file` the way every store under `$SHEP_HOME` is
/// written: staged beside it owner-only and renamed into place, so a reader
/// sees a whole manifest or none.
///
/// The manifest's `defaults` can hold a credential, so nobody else may read
/// the cache.
fn store(file: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    create_cache_dir(dir)?;
    let mut staged = atomic_file::create_staging_file(dir, ".manifest-", ".tmp")?;
    staged.write_all(text.as_bytes())?;
    atomic_file::publish(staged, file)
}

/// Creates `dir` and any missing parents, owner-only on unix at the `mkdir`
/// itself, as the rest of `$SHEP_HOME` is.
fn create_cache_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
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
