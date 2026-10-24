//! What the operator's record says about a dog's access, for a listing to mark.

use serde::{Deserialize, Serialize};

/// The marks `shep flock` and `shep dogs` put on a dog: whether it declared
/// anything, whether the operator has looked at what it was granted, and
/// what it asks for that waits on them.
///
/// Read from `shep.toml` by the CLI that prints the listing. The shepherd
/// never fills it, so a row from any shepherd decodes with
/// [`ProcessInfo::access`](super::ProcessInfo::access) absent.
// wire format: changing this is a breaking change
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DogAccess {
    /// The dog declared nothing and keeps full access.
    pub undeclared: bool,
    /// What the dog holds was written at upgrade and the operator has not yet
    /// re-enabled the dog or run `shep dogs grant` for it.
    pub not_reviewed: bool,
    /// Permissions a newer binary declares that the operator has not
    /// approved. The dog runs on its old grants without them.
    pub waiting: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mark_is_a_key_even_when_it_is_clear() {
        assert_eq!(
            serde_json::to_string(&DogAccess::default()).unwrap(),
            r#"{"undeclared":false,"not_reviewed":false,"waiting":[]}"#
        );
    }

    #[test]
    fn marks_round_trip() {
        let marked = DogAccess {
            undeclared: false,
            not_reviewed: true,
            waiting: vec!["events:process.".to_string()],
        };

        let wire = serde_json::to_string(&marked).unwrap();

        assert_eq!(serde_json::from_str::<DogAccess>(&wire).unwrap(), marked);
    }
}
