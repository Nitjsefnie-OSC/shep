//! [`FlockfileError`]: what a parse refuses with, one variant per reason.

use super::format::FlockFormat;
use core::fmt;

/// Error type returned from [`Flockfile::parse`](crate::config::flockfile::Flockfile::parse).
///
/// `#[non_exhaustive]`: an out-of-tree consumer could otherwise match
/// this exhaustively and a new variant would break them silently. Growth
/// is anticipated per backend, not per format: `.js` Flockfiles never
/// appear here, since shep-core never executes anything. The node bridge
/// lives in shep-cli, which feeds its output back through
/// [`FlockFormat::Json`](crate::config::flockfile::FlockFormat::Json).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlockfileError {
    /// TOML backend rejected the source (carries its message)
    Toml(String),
    /// YAML backend rejected the source
    Yaml(String),
    /// JSON backend rejected the source
    Json(String),
    /// JSON5 backend rejected the source
    Json5(String),
    /// The document parsed but declared no apps
    NoApps,
    /// An `app` entry named one or more keys no field claims.
    ///
    /// Only a key inside an `app` entry lands here. A top-level typo is
    /// refused earlier, by `RawFlockfile`'s own `deny_unknown_fields`,
    /// which the backend's derived `Deserialize` enforces directly rather
    /// than through the `serde_ignored` callback this variant collects
    /// from: it arrives as the format variant
    /// ([`FlockfileError::Toml`], [`FlockfileError::Yaml`],
    /// [`FlockfileError::Json`], [`FlockfileError::Json5`]) naming the key.
    ///
    /// A Flockfile is hand-written, unlike the same [`AppConfig`](crate::config::AppConfig)/
    /// [`ProbeConfig`](crate::config::ProbeConfig) shape riding the wire,
    /// where an unknown field means a newer peer rather than a typo.
    /// `keys` names every offending key (dotted path for a nested one, e.g.
    /// `app.0.readiness_probe.<the misspelled key>`) so one refusal lists
    /// every typo instead of one refusal per run.
    UnknownKeys {
        /// Every key the document named that no field claimed.
        keys: Vec<String>,
    },
}

impl FlockfileError {
    /// The variant for `format`'s backend refusing the source, carrying
    /// `message`.
    ///
    /// The one place a backend refusal is built: a call site names the
    /// `FlockFormat` it asked and the backend's own error, never a variant,
    /// so a fifth format reaches this enum from one match instead of from
    /// every call site.
    #[must_use]
    pub(super) fn backend(format: FlockFormat, message: impl fmt::Display) -> Self {
        let message = message.to_string();
        match format {
            FlockFormat::Toml => Self::Toml(message),
            FlockFormat::Yaml => Self::Yaml(message),
            FlockFormat::Json => Self::Json(message),
            FlockFormat::Json5 => Self::Json5(message),
        }
    }

    /// The format label and message a backend variant carries, `None` for
    /// the variants that name no backend.
    fn backend_parts(&self) -> Option<(&'static str, &str)> {
        match self {
            Self::Toml(m) => Some(("TOML", m)),
            Self::Yaml(m) => Some(("YAML", m)),
            Self::Json(m) => Some(("JSON", m)),
            Self::Json5(m) => Some(("JSON5", m)),
            Self::NoApps | Self::UnknownKeys { .. } => None,
        }
    }
}

impl fmt::Display for FlockfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoApps => f.write_str("Flockfile declares no apps"),
            Self::UnknownKeys { keys } => {
                write!(f, "Flockfile names unrecognized key")?;
                if keys.len() != 1 {
                    f.write_str("s")?;
                }
                write!(f, ": {}", keys.join(", "))
            }
            Self::Toml(_) | Self::Yaml(_) | Self::Json(_) | Self::Json5(_) => {
                // `backend_parts` names exactly these four variants, so the
                // fallback is unreachable; it keeps `Display` exhaustive
                // without a panic on an unreachable path.
                let Some((label, message)) = self.backend_parts() else {
                    return f.write_str("invalid Flockfile");
                };
                write!(f, "invalid {label} Flockfile: {message}")
            }
        }
    }
}

impl core::error::Error for FlockfileError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each backend refusal names its own format, in the one sentence shape
    /// every format variant shares. The strings are what an operator reads
    /// in `shep start` output, so they are pinned whole.
    #[test]
    fn each_backend_variant_displays_its_format_and_message() {
        let cases: [(FlockFormat, FlockfileError, &str); 4] = [
            (
                FlockFormat::Toml,
                FlockfileError::Toml("x".to_string()),
                "invalid TOML Flockfile: x",
            ),
            (
                FlockFormat::Yaml,
                FlockfileError::Yaml("x".to_string()),
                "invalid YAML Flockfile: x",
            ),
            (
                FlockFormat::Json,
                FlockfileError::Json("x".to_string()),
                "invalid JSON Flockfile: x",
            ),
            (
                FlockFormat::Json5,
                FlockfileError::Json5("x".to_string()),
                "invalid JSON5 Flockfile: x",
            ),
        ];
        for (format, variant, rendered) in cases {
            assert_eq!(
                FlockfileError::backend(format, "x"),
                variant,
                "{format:?}: backend built the wrong variant"
            );
            assert_eq!(variant.to_string(), rendered);
        }
    }
}
