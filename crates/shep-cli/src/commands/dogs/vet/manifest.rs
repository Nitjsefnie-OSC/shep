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

/// What a binary's manifest answered. Three answers, in the shape
/// [`super::DogSchema`] uses one flag over: the two ways of having no manifest
/// are different events, and a caller that reads a failed one as an absent
/// declaration lets a dog that declared `validate` go unasked.
// `large_enum_variant` allowed, not fixed: clippy's remedy is to box
// `Manifest`, and that buys nothing. This enum lives for one read of one
// manifest inside one command, its payload is read field by field and
// dropped, and nothing here holds a collection of them.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub(crate) enum ManifestRead {
    /// The manifest, from the cache or from the binary.
    Read(Manifest),
    /// The binary ran and answered nothing: empty output from a run that exited
    /// 0. This is a dog that never heard of [`MANIFEST_FLAG`], which is every
    /// dog adopted before it existed.
    Absent,
    /// There was no answer to read, or what was printed is not a manifest: the
    /// probe outlasted `budget`, exited with a failure, or printed something
    /// other than a manifest. A dog that declared `validate` in a manifest this
    /// read cannot reach still declared it.
    Unreadable,
}

impl ManifestRead {
    /// The manifest, where there is one to read.
    #[must_use]
    pub(crate) fn manifest(&self) -> Option<&Manifest> {
        match self {
            Self::Read(manifest) => Some(manifest),
            Self::Absent | Self::Unreadable => None,
        }
    }
}

/// What `path` declares, from the cache when this exact binary has been read
/// before and from the binary otherwise.
///
/// [`ManifestRead::Absent`] is a dog that answered nothing and exited 0, and
/// [`ManifestRead::Unreadable`] is a probe that gave nothing readable: a run
/// that outlasted `budget`, a run that failed, or output that is not a
/// manifest. Neither is a fault, and neither is cached, so a dog that gains a
/// manifest is noticed on the next call. The cache is derived and safe to
/// delete: one that cannot be read, parsed or written is skipped, and a binary
/// that changed while it was probed is not cached.
///
/// # Errors
/// [`AdoptRefusal::WillNotExec`], and only that, from the probe.
pub(crate) fn read_manifest(
    path: &Path,
    home: &Path,
    name: &str,
    budget: Duration,
) -> Result<ManifestRead, AdoptRefusal> {
    let digest = sha256_hex(path);
    let cached = digest
        .as_ref()
        .map(|digest| home.join(CACHE_DIR).join(format!("{digest}.json")));
    if let Some(manifest) = digest
        .as_deref()
        .and_then(|digest| read_cached(home, digest))
    {
        return Ok(ManifestRead::Read(manifest));
    }
    let Some(text) = ask(path, MANIFEST_FLAG, home, name, budget)? else {
        // No answer at all: still running when the budget ran out, or a run
        // that failed. `docs/dogs.md` asks for a manifest on stdout and exit 0,
        // so a dog that could not answer said nothing about what it wants.
        return Ok(ManifestRead::Unreadable);
    };
    // Empty output is a dog with no manifest, not a manifest that failed to
    // parse: the same guard `ask_schema` makes, so the ordinary case earns no
    // reading meant for a broken one.
    if text.trim().is_empty() {
        return Ok(ManifestRead::Absent);
    }
    let Ok(manifest) = serde_json::from_str::<Manifest>(&text) else {
        return Ok(ManifestRead::Unreadable);
    };
    // A binary swapped while it ran may have answered as the new build, and
    // the old digest would file that answer under the wrong key.
    if let Some(file) = cached.filter(|_| sha256_hex(path) == digest) {
        let _ = store(&file, &text);
    }
    Ok(ManifestRead::Read(manifest))
}

/// What the cache already holds for `path`, and nothing else: this never
/// probes the binary.
///
/// [`read_manifest`] asks a dog that has not been asked before. Nothing here
/// does, because `shep --help` and `shep completions` run on whatever the
/// operator already adopted and would otherwise spawn a dog to answer a
/// question about how to spell its name.
///
/// `None` is ordinary: a dog adopted before manifests existed, one that
/// answers no manifest, one whose recorded binary has since moved, and a
/// cache file that will not parse all answer it.
pub(crate) fn cached_manifest(path: &Path, home: &Path) -> Option<Manifest> {
    read_cached(home, &sha256_hex(path)?)
}

/// The manifest filed under `digest`, or `None`.
fn read_cached(home: &Path, digest: &str) -> Option<Manifest> {
    let file = home.join(CACHE_DIR).join(format!("{digest}.json"));
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
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
