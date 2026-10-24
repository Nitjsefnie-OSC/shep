//! The read side of `[daemon.dog_grants]`, where an operator records what a
//! dog may ask of the shepherd.

use shep_core::permissions::Grants;

use super::ShepToml;

impl ShepToml {
    /// The permissions recorded for `name` under
    /// `[daemon.dog_grants.<name>] granted`, or `None` when the dog has no
    /// entry and is undeclared, with full access.
    ///
    /// Names are taken as written, unchecked: the shepherd refuses a name it
    /// cannot read at boot, naming it, and a reader here must not hide that.
    #[must_use]
    pub fn granted(&self, name: &str) -> Option<Grants> {
        let names = self
            .table("daemon")?
            .get("dog_grants")?
            .as_table()?
            .get(name)?
            .as_table()?
            .get("granted")?
            .as_array()?;
        Some(Grants::from_names(
            names.iter().filter_map(|v| v.as_str().map(str::to_owned)),
        ))
    }
}
