//! [`Trigger::LogMatch`](super::Trigger::LogMatch): the stream a rule reads,
//! its compiled pattern, and the message a matching line becomes.

use serde::Deserialize;

use super::RulesError;

/// Largest compiled program a `log_match` pattern may produce: 1 MiB, the
/// bound `LevelMatcher::compile` puts on a pattern from a Flockfile.
const PATTERN_SIZE_LIMIT: usize = 1 << 20;

/// Most bytes of a matched line a message carries.
///
/// Discord rejects a `content` over 2,000 characters, and the ring that
/// holds `barks.jsonl` should not take a megabyte for one alert.
const LINE_CAP: usize = 1_000;

/// Which of a sheep's output streams a `log_match` rule reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    /// Standard output only.
    Out,
    /// Standard error only.
    Err,
    /// Either.
    #[default]
    Both,
}

impl LogStream {
    fn reads_out(self) -> bool {
        matches!(self, Self::Out | Self::Both)
    }

    fn reads_err(self) -> bool {
        matches!(self, Self::Err | Self::Both)
    }
}

/// Which stream one log frame came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Source {
    Out,
    Err,
}

impl Source {
    /// The word a message uses for it.
    fn word(self) -> &'static str {
        match self {
            Self::Out => "stdout",
            Self::Err => "stderr",
        }
    }
}

/// One `log_match` rule, compiled once in [`Rules::new`](super::Rules::new).
#[derive(Debug)]
pub(super) struct LogRule {
    /// Position in the configured rule list.
    pub(super) idx: usize,
    stream: LogStream,
    regex: regex::Regex,
}

impl LogRule {
    /// Compiles rule `index`'s `pattern`.
    pub(super) fn compile(
        index: usize,
        pattern: &str,
        stream: LogStream,
    ) -> Result<Self, RulesError> {
        let refused = |reason: String| RulesError::BadPattern { index, reason };
        if pattern.is_empty() {
            return Err(refused(
                "the pattern is empty and would match every line".to_owned(),
            ));
        }
        let regex = regex::RegexBuilder::new(pattern)
            .size_limit(PATTERN_SIZE_LIMIT)
            .build()
            .map_err(|err| refused(err.to_string()))?;
        Ok(Self {
            idx: index,
            stream,
            regex,
        })
    }

    /// Whether this rule reads `source` and `line` matches its pattern.
    pub(super) fn matches(&self, source: Source, line: &str) -> bool {
        let reads = match source {
            Source::Out => self.stream.reads_out(),
            Source::Err => self.stream.reads_err(),
        };
        reads && self.regex.is_match(line)
    }

    /// The bus topics the rules in `rules` need, `log.out` before `log.err`.
    pub(super) fn topics(rules: &[Self]) -> Vec<String> {
        let mut topics = Vec::new();
        if rules.iter().any(|rule| rule.stream.reads_out()) {
            topics.push("log.out".to_owned());
        }
        if rules.iter().any(|rule| rule.stream.reads_err()) {
            topics.push("log.err".to_owned());
        }
        topics
    }
}

/// What a matching `line` from `name`'s `source` says, the line cut to
/// [`LINE_CAP`] bytes at a char boundary.
pub(super) fn message(name: &str, source: Source, line: &str) -> String {
    let mut end = line.len().min(LINE_CAP);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    format!("{name} logged to {}: {}", source.word(), &line[..end])
}
