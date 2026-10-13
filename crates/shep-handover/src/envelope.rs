//! The envelope around the state: its layout, and reading it back.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::State;

/// The layout of the envelope around the state, not of the state itself.
pub(crate) const ENVELOPE_VERSION: u32 = 1;

/// What an outgoing image writes: the state under its format, and the number
/// each carried descriptor holds in the new image, by key.
#[derive(Serialize)]
struct Outbound<'a, S> {
    envelope: u32,
    format: u32,
    fds: &'a BTreeMap<String, i32>,
    state: &'a S,
}

/// The blob for `state`, as bytes.
///
/// # Errors
///
/// The state does not serialize to JSON.
pub(crate) fn encode<S: State>(
    fds: &BTreeMap<String, i32>,
    state: &S,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&Outbound {
        envelope: ENVELOPE_VERSION,
        format: S::FORMAT,
        fds,
        state,
    })
}

/// A blob read back: the descriptor numbers, and the state if this build can
/// read its format.
pub(crate) struct Inbound<S> {
    pub fds: BTreeMap<String, i32>,
    pub state: Result<S, Unreadable>,
}

/// Why a blob's state was not read, once its descriptors were.
pub(crate) enum Unreadable {
    /// The state is in a format this build does not read.
    Format { found: Option<u64>, supported: u32 },
    /// The state is in a format this build reads but does not fit its type.
    Malformed(serde_json::Error),
}

/// Why a blob's envelope was not read at all.
pub(crate) enum EnvelopeError {
    /// Not JSON, or not an envelope.
    Malformed(serde_json::Error),
    /// An envelope layout this build does not implement.
    Version(Option<u64>),
}

/// Read a blob.
///
/// The envelope version and the format are read off the raw JSON before the
/// state is deserialized, so an unknown format is refused by number instead
/// of by a field that failed to deserialize.
pub(crate) fn decode<S: State>(bytes: &[u8]) -> Result<Inbound<S>, EnvelopeError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(EnvelopeError::Malformed)?;
    let version = value.get("envelope").and_then(serde_json::Value::as_u64);
    if version != Some(u64::from(ENVELOPE_VERSION)) {
        return Err(EnvelopeError::Version(version));
    }
    let fds = match value.get("fds") {
        Some(fds) => serde_json::from_value(fds.clone()).map_err(EnvelopeError::Malformed)?,
        None => BTreeMap::new(),
    };
    let found = value.get("format").and_then(serde_json::Value::as_u64);
    let state = match found {
        Some(found) if u32::try_from(found).is_ok_and(S::reads) => {
            serde_json::from_value(value.get("state").cloned().unwrap_or_default())
                .map_err(Unreadable::Malformed)
        }
        found => Err(Unreadable::Format {
            found,
            supported: S::FORMAT,
        }),
    };
    Ok(Inbound { fds, state })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Two {
        n: u32,
    }

    impl State for Two {
        const FORMAT: u32 = 2;
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct ReadsOneAndTwo {
        n: u32,
    }

    impl State for ReadsOneAndTwo {
        const FORMAT: u32 = 2;

        fn reads(format: u32) -> bool {
            matches!(format, 1 | 2)
        }
    }

    fn blob(format: u32, state: &str) -> Vec<u8> {
        format!(r#"{{"envelope":1,"format":{format},"fds":{{"out":5}},"state":{state}}}"#)
            .into_bytes()
    }

    fn fds(pairs: &[(&str, i32)]) -> BTreeMap<String, i32> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    #[test]
    fn a_blob_written_in_this_format_reads_back() {
        let bytes = encode(&fds(&[("out", 5)]), &Two { n: 9 }).unwrap();
        let inbound = decode::<Two>(&bytes).ok().unwrap();
        assert_eq!(inbound.fds, fds(&[("out", 5)]));
        assert_eq!(inbound.state.ok(), Some(Two { n: 9 }));
    }

    #[test]
    fn the_envelope_is_the_documented_json() {
        let bytes = encode(&fds(&[("out", 5)]), &Two { n: 9 }).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"envelope":1,"format":2,"fds":{"out":5},"state":{"n":9}}"#
        );
    }

    #[test]
    fn a_format_this_build_cannot_read_is_refused_by_number() {
        let inbound = decode::<Two>(&blob(3, r#"{"unrelated":true}"#))
            .ok()
            .unwrap();
        assert!(matches!(
            inbound.state,
            Err(Unreadable::Format {
                found: Some(3),
                supported: 2
            })
        ));
        assert_eq!(
            inbound.fds,
            fds(&[("out", 5)]),
            "the descriptors still come back, to be closed"
        );
    }

    #[test]
    fn a_state_that_reads_two_formats_accepts_the_older_one() {
        let inbound = decode::<ReadsOneAndTwo>(&blob(1, r#"{"n":4}"#))
            .ok()
            .unwrap();
        assert_eq!(inbound.state.ok(), Some(ReadsOneAndTwo { n: 4 }));
    }

    #[test]
    fn a_blob_with_no_format_is_refused() {
        let bytes = br#"{"envelope":1,"fds":{},"state":{"n":1}}"#;
        let inbound = decode::<Two>(bytes).ok().unwrap();
        assert!(matches!(inbound.state, Err(Unreadable::Format { .. })));
    }

    #[test]
    fn an_envelope_layout_from_another_line_is_refused() {
        let bytes = br#"{"envelope":2,"format":2,"state":{"n":1}}"#;
        assert!(matches!(
            decode::<Two>(bytes),
            Err(EnvelopeError::Version(Some(2)))
        ));
        assert!(matches!(
            decode::<Two>(br#"{"format":2}"#),
            Err(EnvelopeError::Version(None))
        ));
    }

    #[test]
    fn bytes_that_are_not_json_are_malformed() {
        assert!(matches!(
            decode::<Two>(b"not json"),
            Err(EnvelopeError::Malformed(_))
        ));
    }

    #[test]
    fn a_state_that_does_not_fit_its_type_is_malformed_not_a_format_refusal() {
        let inbound = decode::<Two>(&blob(2, r#"{"n":"nine"}"#)).ok().unwrap();
        assert!(matches!(inbound.state, Err(Unreadable::Malformed(_))));
    }
}
