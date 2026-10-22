//! Rows for `shep host`'s list of machines.

use serde::Serialize;

use crate::output::Render;

/// One entry of `shep.toml`'s `hosts` list.
#[derive(Debug, Serialize)]
pub struct HostEntry {
    /// The ssh destination, as written in `shep.toml`.
    pub host: String,
}

/// `shep host`'s list, in file order.
///
/// `transparent`, for [`KvRows`](super::KvRows)' reason: a plain array of
/// [`HostEntry`] objects. Never from a `Response`: the list never touches
/// the wire.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct HostRows(pub Vec<HostEntry>);

/// No colour: a destination is operator data, and shep has no opinion about
/// it.
impl Render for HostRows {
    fn headers() -> &'static [&'static str] {
        &["HOST"]
    }

    fn rows(&self) -> Vec<Vec<String>> {
        self.0
            .iter()
            .map(|entry| vec![entry.host.clone()])
            .collect()
    }

    /// # Panics
    /// If `header` is not one of `Self::headers()`'s own values.
    #[track_caller]
    fn json_key_for(header: &str) -> &'static str {
        match header {
            "HOST" => "host",
            other => panic!("HostRows::headers() does not include {other:?}"),
        }
    }

    const JSON_ONLY: &'static [&'static str] = &[];

    // One column: a row with no host is not a row.
    const PRIORITIES: &'static [u8] = &[0];
}

#[cfg(test)]
mod tests {
    use super::super::tests::assert_no_drift;
    use super::*;

    #[test]
    fn host_rows_match_their_json_keys() {
        let rows = HostRows(vec![
            HostEntry {
                host: "amp2".to_string(),
            },
            HostEntry {
                host: "deploy@build".to_string(),
            },
        ]);
        assert_no_drift(&rows, |json| &json[0], &[]);
    }
}
