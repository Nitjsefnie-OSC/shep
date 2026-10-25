//! The one read shared by the versioned stores under `$SHEP_HOME`.
//!
//! `kv.json`, `overrides.json` and `secrets.json` are each a JSON object
//! with a `version` field. All three read the same way: a missing file is
//! the empty, current-version store, and a file naming a version this build
//! does not understand is refused rather than rewritten. A migration step
//! changes that decision once, here.
//!
//! Writing is not shared: each store stages through [`crate::atomic_file`]
//! under the [`crate::file_lock`] its own callers hold.

use core::fmt;
use std::path::Path;

use serde::de::DeserializeOwned;

/// The `version` field a store's on-disk shape carries.
pub(crate) trait Versioned {
    /// The format version the value read from disk declares.
    fn version(&self) -> u32;
}

/// A read that did not produce a store this build can use.
///
/// Carries no file content: `FutureVersion` holds the number alone, so
/// nothing here needs redaction for a store that holds secrets.
#[derive(Debug)]
pub(crate) enum ReadError {
    /// The file could not be opened or read. A file that is simply absent
    /// is not this: it reads as the type's default.
    Io(std::io::Error),
    /// The file is not the JSON shape the store writes.
    Decode(serde_json::Error),
    /// The file names a version above the caller's own; carries it. Nothing
    /// was written.
    FutureVersion(u32),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "store read failed: {err}"),
            Self::Decode(err) => write!(f, "store failed to parse: {err}"),
            Self::FutureVersion(version) => {
                write!(
                    f,
                    "store is version {version}, newer than this build understands"
                )
            }
        }
    }
}

impl core::error::Error for ReadError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Decode(err) => Some(err),
            Self::FutureVersion(_) => None,
        }
    }
}

/// Reads `path` as a `T`, where `current` is the version this build writes.
///
/// # Errors
///
/// - [`ReadError::Io`]: the file could not be opened or read. A file that is
///   absent is not an error: it reads as `T::default()`.
/// - [`ReadError::Decode`]: the file is not the JSON shape `T` is written as.
/// - [`ReadError::FutureVersion`]: the file's `version` is above `current`.
pub(crate) fn read_versioned<T>(path: &Path, current: u32) -> Result<T, ReadError>
where
    T: Default + DeserializeOwned + Versioned,
{
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(err) => return Err(ReadError::Io(err)),
    };
    let file: T = serde_json::from_str(&raw).map_err(ReadError::Decode)?;
    if file.version() > current {
        return Err(ReadError::FutureVersion(file.version()));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// A store shape for these tests only. The three real ones are pinned by
    /// their own modules' tests, through their own public functions.
    #[derive(Debug, Default, Deserialize)]
    struct Fixture {
        version: u32,
        notes: Vec<String>,
    }

    impl Versioned for Fixture {
        fn version(&self) -> u32 {
            self.version
        }
    }

    #[test]
    fn a_missing_file_reads_as_default() {
        let dir = tempfile::tempdir().unwrap();
        let file: Fixture = read_versioned(&dir.path().join("store.json"), 1).unwrap();
        assert_eq!(file.version, 0);
        assert!(file.notes.is_empty());
    }

    #[test]
    fn a_version_equal_to_current_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        std::fs::write(&path, r#"{"version":1,"notes":["a"]}"#).unwrap();

        let file: Fixture = read_versioned(&path, 1).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.notes, ["a".to_string()]);
    }

    #[test]
    fn a_version_above_current_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        std::fs::write(&path, r#"{"version":9,"notes":["keep me"]}"#).unwrap();

        let err = read_versioned::<Fixture>(&path, 1).unwrap_err();
        assert!(matches!(err, ReadError::FutureVersion(9)), "{err:?}");
        // Untouched: refusing is the only thing a reader may do with it.
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"version":9,"notes":["keep me"]}"#
        );
    }

    #[test]
    fn malformed_json_is_a_decode_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        std::fs::write(&path, "{\"version\":1,").unwrap();

        let err = read_versioned::<Fixture>(&path, 1).unwrap_err();
        assert!(matches!(err, ReadError::Decode(_)), "{err:?}");
        assert!(
            core::error::Error::source(&err).is_some(),
            "the parser's own diagnostic survives"
        );
    }

    #[test]
    fn a_directory_in_place_of_the_file_is_an_io_error() {
        // Not `NotFound`, so it must not be mistaken for an absent store and
        // silently replaced by a write of the default.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        std::fs::create_dir(&path).unwrap();

        let err = read_versioned::<Fixture>(&path, 1).unwrap_err();
        assert!(matches!(err, ReadError::Io(_)), "{err:?}");
    }
}
