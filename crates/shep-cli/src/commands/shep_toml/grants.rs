//! The read side of `[daemon.dog_grants]`, where an operator records what a
//! dog may ask of the shepherd.

use shep_core::permissions::Grants;
use toml_edit::Item;

use super::{ShepToml, ShepTomlError};

impl ShepToml {
    /// The permissions recorded for `name` under
    /// `[daemon.dog_grants.<name>] granted`, or `None` when the dog has no
    /// entry and is undeclared, with full access.
    ///
    /// Read as strictly as the shepherd reads it at boot, so a hand-typed
    /// typo is refused where the operator is looking rather than recorded as
    /// a grant that matches nothing.
    ///
    /// # Errors
    /// [`ShepTomlError::BadGrants`] when the entry is not a table holding
    /// only a `granted` array of strings, or names a permission shep cannot
    /// read.
    pub fn granted(&self, name: &str) -> Result<Option<Grants>, ShepTomlError> {
        // Table-like, not `as_table`: `dog_grants = { otel = { granted = [] } }`
        // is the same TOML as the headed form and the boot reads both, so a
        // reader that saw only headed tables would send a dog granted nothing
        // as undeclared, with full access.
        let Some(entry) = self
            .doc
            .get("daemon")
            .and_then(Item::as_table_like)
            .and_then(|daemon| daemon.get("dog_grants"))
            .and_then(Item::as_table_like)
            .and_then(|grants| grants.get(name))
        else {
            return Ok(None);
        };
        let bad = |reason: String| ShepTomlError::BadGrants {
            path: self.path.clone(),
            dog: name.to_string(),
            reason,
        };
        let entry = entry
            .as_table_like()
            .ok_or_else(|| bad("must be a table holding `granted`".to_string()))?;
        // The boot's `DogGrantsSection` denies unknown keys, so a stray one
        // is refused here, where the operator is looking, and not at the next
        // boot.
        if let Some((key, _)) = entry.iter().find(|(key, _)| *key != "granted") {
            return Err(bad(format!("`{key}` is not a key shep reads here")));
        }
        let names = entry
            .get("granted")
            .and_then(Item::as_array)
            .and_then(|names| names.iter().map(|v| v.as_str()).collect::<Option<Vec<_>>>())
            .ok_or_else(|| bad("`granted` must be a list of permission names".to_string()))?;
        Grants::parse(names)
            .map(Some)
            .map_err(|err| bad(err.to_string()))
    }
}
