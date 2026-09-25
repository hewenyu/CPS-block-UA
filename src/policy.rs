use gateway_plugin_sdk::call::middleware::MiddlewareHeader;
use regex::{RegexBuilder, RegexSet, RegexSetBuilder};
use serde::Deserialize;
use serde_json::Value;

pub const DEFAULT_PATTERN: &str = r"(?:codex_cli_rs|codex-cli|codex-tui|codex_exec|codex_vscode|Codex Desktop)/[0-9]+(?:\.[0-9]+){1,3}(?:[-+][0-9A-Za-z.-]+)?(?: [ -~]*)?";
const MAX_UA_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Observe,
    Enforce,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Enforce => "enforce",
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Configuration {
    mode: Mode,
    allow_patterns: Vec<String>,
    allow_models: bool,
}

impl Default for Configuration {
    fn default() -> Self {
        Self {
            mode: Mode::Observe,
            allow_patterns: vec![DEFAULT_PATTERN.to_owned()],
            allow_models: true,
        }
    }
}

// Configuration errors deliberately omit supplied values and regex source.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error(
        "configuration must be an object with mode, allow_patterns and/or allow_models of the documented types"
    )]
    Shape,
    #[error("allow_patterns must contain 1 to 32 nonempty patterns of at most 1024 bytes each")]
    PatternLimits,
    #[error("allow_patterns contains an invalid regex or exceeds the regex compilation budget")]
    PatternSyntax,
}

pub struct Policy {
    mode: Mode,
    patterns: RegexSet,
    allow_models: bool,
}

impl Policy {
    pub fn from_value(value: Value) -> Result<Self, ConfigError> {
        // Serde structs can otherwise accept positional arrays, including [] with defaults.
        if !value.is_object() {
            return Err(ConfigError::Shape);
        }
        let config: Configuration =
            serde_json::from_value(value).map_err(|_| ConfigError::Shape)?;
        if config.allow_patterns.is_empty()
            || config.allow_patterns.len() > 32
            || config
                .allow_patterns
                .iter()
                .any(|p| p.is_empty() || p.len() > 1024)
        {
            return Err(ConfigError::PatternLimits);
        }
        // Validate each expression alone before adding our grouping and anchors. Otherwise
        // an unbalanced expression such as `foo)|(?:bar` can escape that grouping.
        for pattern in &config.allow_patterns {
            RegexBuilder::new(pattern)
                .size_limit(2 * 1024 * 1024)
                .dfa_size_limit(2 * 1024 * 1024)
                .build()
                .map_err(|_| ConfigError::PatternSyntax)?;
        }
        let patterns = RegexSetBuilder::new(
            config
                .allow_patterns
                .iter()
                .map(|p| format!(r"\A(?:{p})\z")),
        )
        .size_limit(2 * 1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .build()
        .map_err(|_| ConfigError::PatternSyntax)?;
        Ok(Self {
            mode: config.mode,
            patterns,
            allow_models: config.allow_models,
        })
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// None means allowed; a reason means reject (or would reject in observe mode).
    /// The caller forwards the original request, including its unmodified headers.
    pub fn rejection_reason(
        &self,
        endpoint: &str,
        headers: &[MiddlewareHeader],
    ) -> Option<&'static str> {
        if self.allow_models && is_model_query(endpoint) {
            return None;
        }
        let mut values = headers
            .iter()
            .filter(|h| h.name.eq_ignore_ascii_case("user-agent"));
        let Some(first) = values.next() else {
            return Some("missing_ua");
        };
        if values.next().is_some() {
            return Some("duplicate_ua");
        }
        if first.value.len() > MAX_UA_BYTES {
            return Some("oversized_ua");
        }
        let Ok(ua) = std::str::from_utf8(&first.value) else {
            return Some("invalid_ua");
        };
        // HTTP optional whitespace is SP/HTAB, not arbitrary Unicode whitespace.
        let ua = ua.trim_matches([' ', '\t']);
        if ua.is_empty() {
            return Some("empty_ua");
        }
        if !ua.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return Some("invalid_ua");
        }
        if self.patterns.is_match(ua) {
            None
        } else {
            Some("unmatched_ua")
        }
    }
}

fn is_model_query(endpoint: &str) -> bool {
    endpoint == "/v1/models"
        || endpoint.strip_prefix("/v1/models/").is_some_and(|id| {
            !id.is_empty() && !id.contains(['/', '?', '#', '%']) && id != "." && id != ".."
        })
}
