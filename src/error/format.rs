use std::fmt;

use serde::Serialize;

use crate::error::{Error, ErrorCode as _};

/// The top-level JSON body returned by API endpoints on error.
///
/// Serializes to `{"error": { … }}` so that consumers can always key on
/// `error.code` regardless of HTTP status.
#[derive(Debug, Serialize)]
pub struct JsonErrorBody {
    pub error: JsonErrorDetail,
}

/// The detail object nested inside [`JsonErrorBody`].
#[derive(Debug, Serialize)]
pub struct JsonErrorDetail {
    /// Stable `SCREAMING_SNAKE_CASE` identifier — safe for programmatic use.
    pub code: &'static str,
    /// Human-readable description of what went wrong.
    pub message: String,
    /// Actionable step the operator can take to resolve the issue.
    pub suggestion: String,
    /// Full debug chain of error sources; omitted in production responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<String>,
}

impl Error {
    /// Serialize the error into a [`JsonErrorBody`].
    ///
    /// When `include_debug` is `true` the `debug` field is populated with the
    /// full `Display` chain of the error and all of its sources.
    pub fn to_json_body(&self, include_debug: bool) -> JsonErrorBody {
        let debug = if include_debug {
            Some(build_debug_chain(self))
        } else {
            None
        };

        JsonErrorBody {
            error: JsonErrorDetail {
                code: self.code(),
                message: self.to_string(),
                suggestion: self.suggestion(),
                debug,
            },
        }
    }
}

/// Walk the full [`std::error::Error::source`] chain and produce a single
/// newline-separated string showing every cause.
fn build_debug_chain(err: &Error) -> String {
    use std::error::Error as StdError;

    let mut parts = vec![err.to_string()];
    let mut source: Option<&dyn StdError> = err.source();
    while let Some(s) = source {
        parts.push(format!("caused by: {s}"));
        source = s.source();
    }
    parts.join("\n")
}

/// Newtype wrapper around [`Error`] that renders a CLI-friendly one-liner.
///
/// Format:
/// ```text
/// error[CODE]: message
///   -> suggestion
/// ```
pub struct CliFormatter<'a>(pub &'a Error);

impl fmt::Display for CliFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let err = self.0;
        write!(
            f,
            "error[{code}]: {message}\n  -> {suggestion}",
            code = err.code(),
            message = err,
            suggestion = err.suggestion(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    fn sample_not_found() -> Error {
        Error::AgentNotFound {
            name: "my-agent".to_owned(),
        }
    }

    fn sample_daemon() -> Error {
        Error::DaemonNotRunning
    }

    // ── JSON serialization ────────────────────────────────────────────────────

    #[test]
    fn test_json_body_round_trip_no_debug() {
        let err = sample_not_found();
        let body = err.to_json_body(false);
        let json = serde_json::to_value(&body).expect("serialization must succeed");

        assert_eq!(json["error"]["code"], "AGENT_NOT_FOUND");
        assert!(!json["error"]["message"].as_str().unwrap().is_empty());
        assert!(!json["error"]["suggestion"].as_str().unwrap().is_empty());
        assert!(
            json["error"].get("debug").is_none(),
            "debug must be absent when include_debug is false"
        );
    }

    #[test]
    fn test_json_body_includes_debug_field_when_requested() {
        let err = sample_daemon();
        let body = err.to_json_body(true);
        let json = serde_json::to_value(&body).expect("serialization must succeed");

        assert!(
            json["error"].get("debug").is_some(),
            "debug must be present when include_debug is true"
        );
        assert!(!json["error"]["debug"].as_str().unwrap().is_empty());
    }

    #[test]
    fn test_json_body_debug_absent_when_false() {
        let body = sample_daemon().to_json_body(false);
        let json = serde_json::to_value(&body).expect("serialization must succeed");
        assert!(json["error"].get("debug").is_none());
    }

    // ── CLI formatter ─────────────────────────────────────────────────────────

    #[test]
    fn test_cli_formatter_matches_expected_format() {
        let err = sample_not_found();
        let rendered = CliFormatter(&err).to_string();

        let code = err.code();
        let message = err.to_string();
        let suggestion = err.suggestion();

        let expected = format!("error[{code}]: {message}\n  -> {suggestion}");
        assert_eq!(rendered, expected);
    }

    #[test]
    fn test_cli_formatter_starts_with_error_prefix() {
        let err = sample_daemon();
        let rendered = CliFormatter(&err).to_string();
        assert!(
            rendered.starts_with("error["),
            "CLI output must start with 'error[', got: {rendered:?}"
        );
    }

    #[test]
    fn test_cli_formatter_contains_arrow_suggestion() {
        let err = sample_daemon();
        let rendered = CliFormatter(&err).to_string();
        assert!(
            rendered.contains("\n  -> "),
            "CLI output must contain suggestion line, got: {rendered:?}"
        );
    }
}
