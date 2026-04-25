mod code;
mod format;

pub use code::ErrorCode;
pub use format::{CliFormatter, JsonErrorBody, JsonErrorDetail};

use axum::http::StatusCode;
use thiserror::Error;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// All structured errors produced by the Trumpet daemon and CLI.
///
/// The enum is intentionally flat — one level, no nesting — so that match
/// arms stay simple and `ErrorCode` implementations remain straightforward.
#[derive(Debug, Error)]
pub enum Error {
    // ── Daemon ────────────────────────────────────────────────────────────────
    /// The daemon process is not running; most commands require it.
    #[error("daemon is not running")]
    DaemonNotRunning,

    /// A daemon is already running with the given PID.
    #[error("daemon is already running (pid {pid})")]
    DaemonAlreadyRunning { pid: u32 },

    /// A stale socket exists at `path` left behind by a dead process.
    #[error("stale daemon socket at '{path}' (was pid {pid})")]
    DaemonStaleSocket { path: String, pid: u32 },

    /// The daemon could not bind to its socket.
    #[error("daemon failed to bind socket at '{path}': {reason}")]
    DaemonBindFailed { path: String, reason: String },

    /// The daemon could not shut down cleanly.
    #[error("daemon shutdown failed: {reason}")]
    DaemonShutdownFailed { reason: String },

    // ── Connection ────────────────────────────────────────────────────────────
    /// The connection to the daemon was actively refused.
    #[error("connection refused by daemon")]
    ConnectionRefused,

    /// The connection attempt timed out.
    #[error("connection timed out after {duration}")]
    ConnectionTimeout { duration: String },

    /// No socket file exists at the expected path.
    #[error("daemon socket not found at '{path}'")]
    ConnectionSocketNotFound { path: String },

    // ── Agent ─────────────────────────────────────────────────────────────────
    /// No agent with this name is registered.
    #[error("agent '{name}' not found")]
    AgentNotFound { name: String },

    /// An agent with this name is already registered.
    #[error("agent '{name}' is already registered")]
    AgentAlreadyRegistered { name: String },

    /// The provided agent name is syntactically invalid.
    #[error("agent name '{name}' is invalid: {reason}")]
    AgentInvalidName { name: String, reason: String },

    // ── Conversation ──────────────────────────────────────────────────────────
    /// No conversation with this ID exists.
    #[error("conversation '{id}' not found")]
    ConversationNotFound { id: String },

    /// The agent is not a participant in the conversation.
    #[error("agent '{agent}' is not a participant in conversation '{id}'")]
    ConversationNotParticipant { agent: String, id: String },

    /// A conversation cannot be created without at least one participant.
    #[error("conversation must have at least one participant")]
    ConversationEmptyParticipants,

    // ── Skill ─────────────────────────────────────────────────────────────────
    /// No skill with this name is registered.
    #[error("skill '{name}' not found")]
    SkillNotFound { name: String },

    /// No skill with this ID is registered.
    #[error("skill with id '{id}' not found")]
    SkillNotFoundById { id: String },

    /// A skill with this name is already registered.
    #[error("skill '{name}' is already registered")]
    SkillAlreadyRegistered { name: String },

    /// The provided skill name is syntactically invalid.
    #[error("skill name '{name}' is invalid: {reason}")]
    SkillInvalidName { name: String, reason: String },

    /// The provided skill description is empty or invalid.
    #[error("skill '{name}' has invalid description: {reason}")]
    SkillInvalidDescription { name: String, reason: String },

    /// Skill invocation failed.
    #[error("skill invocation failed for '{name}': {reason}")]
    SkillInvocationFailed { name: String, reason: String },

    /// Skill invocation not yet implemented.
    #[error("skill invocation not yet implemented for '{name}'")]
    SkillNotImplemented { name: String },

    /// The agent providing the skill is not connected.
    #[error("skill provider unavailable for '{name}'")]
    SkillProviderUnavailable { name: String },

    // ── Config ────────────────────────────────────────────────────────────────
    /// The config file contains invalid TOML.
    #[error("invalid TOML in config file '{path}': {reason}")]
    ConfigInvalidToml { path: String, reason: String },

    /// The expected config directory does not exist.
    #[error("config directory not found: '{path}'")]
    ConfigMissingDir { path: String },

    /// The process lacks permission to read or write the config path.
    #[error("permission denied accessing config at '{path}'")]
    ConfigPermissionDenied { path: String },

    /// The config values are syntactically valid but semantically incorrect.
    #[error("config validation failed: {reason}")]
    ConfigValidationFailed { reason: String },

    // ── Internal ──────────────────────────────────────────────────────────────
    /// An unexpected internal condition was reached.
    #[error("internal error: {reason}")]
    InternalUnexpected { reason: String },

    /// The internal message bus is at capacity.
    #[error("internal message bus is full (capacity: {capacity})")]
    InternalBusFull { capacity: usize },
}

impl ErrorCode for Error {
    fn code(&self) -> &'static str {
        match self {
            Self::DaemonNotRunning => "DAEMON_NOT_RUNNING",
            Self::DaemonAlreadyRunning { .. } => "DAEMON_ALREADY_RUNNING",
            Self::DaemonStaleSocket { .. } => "DAEMON_STALE_SOCKET",
            Self::DaemonBindFailed { .. } => "DAEMON_BIND_FAILED",
            Self::DaemonShutdownFailed { .. } => "DAEMON_SHUTDOWN_FAILED",
            Self::ConnectionRefused => "CONNECTION_REFUSED",
            Self::ConnectionTimeout { .. } => "CONNECTION_TIMEOUT",
            Self::ConnectionSocketNotFound { .. } => "CONNECTION_SOCKET_NOT_FOUND",
            Self::AgentNotFound { .. } => "AGENT_NOT_FOUND",
            Self::AgentAlreadyRegistered { .. } => "AGENT_ALREADY_REGISTERED",
            Self::AgentInvalidName { .. } => "AGENT_INVALID_NAME",
            Self::ConversationNotFound { .. } => "CONVERSATION_NOT_FOUND",
            Self::ConversationNotParticipant { .. } => "CONVERSATION_NOT_PARTICIPANT",
            Self::ConversationEmptyParticipants => "CONVERSATION_EMPTY_PARTICIPANTS",
            Self::SkillNotFound { .. } => "SKILL_NOT_FOUND",
            Self::SkillNotFoundById { .. } => "SKILL_NOT_FOUND_BY_ID",
            Self::SkillAlreadyRegistered { .. } => "SKILL_ALREADY_REGISTERED",
            Self::SkillInvalidName { .. } => "SKILL_INVALID_NAME",
            Self::SkillInvalidDescription { .. } => "SKILL_INVALID_DESCRIPTION",
            Self::SkillInvocationFailed { .. } => "SKILL_INVOCATION_FAILED",
            Self::SkillNotImplemented { .. } => "SKILL_NOT_IMPLEMENTED",
            Self::SkillProviderUnavailable { .. } => "SKILL_PROVIDER_UNAVAILABLE",
            Self::ConfigInvalidToml { .. } => "CONFIG_INVALID_TOML",
            Self::ConfigMissingDir { .. } => "CONFIG_MISSING_DIR",
            Self::ConfigPermissionDenied { .. } => "CONFIG_PERMISSION_DENIED",
            Self::ConfigValidationFailed { .. } => "CONFIG_VALIDATION_FAILED",
            Self::InternalUnexpected { .. } => "INTERNAL_UNEXPECTED",
            Self::InternalBusFull { .. } => "INTERNAL_BUS_FULL",
        }
    }

    fn http_status(&self) -> StatusCode {
        match self {
            Self::DaemonNotRunning
            | Self::DaemonShutdownFailed { .. }
            | Self::SkillProviderUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::DaemonAlreadyRunning { .. }
            | Self::AgentAlreadyRegistered { .. }
            | Self::SkillAlreadyRegistered { .. }
            | Self::DaemonStaleSocket { .. } => StatusCode::CONFLICT,
            Self::DaemonBindFailed { .. }
            | Self::ConnectionRefused
            | Self::ConnectionTimeout { .. }
            | Self::ConnectionSocketNotFound { .. } => StatusCode::BAD_GATEWAY,
            Self::SkillInvocationFailed { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            Self::AgentNotFound { .. }
            | Self::ConversationNotFound { .. }
            | Self::SkillNotFound { .. }
            | Self::SkillNotFoundById { .. }
            | Self::ConfigMissingDir { .. } => StatusCode::NOT_FOUND,
            Self::SkillNotImplemented { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::AgentInvalidName { .. }
            | Self::SkillInvalidName { .. }
            | Self::SkillInvalidDescription { .. }
            | Self::ConversationEmptyParticipants
            | Self::ConfigInvalidToml { .. }
            | Self::ConfigValidationFailed { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::ConversationNotParticipant { .. } => StatusCode::FORBIDDEN,
            Self::ConfigPermissionDenied { .. } => StatusCode::FORBIDDEN,
            Self::InternalUnexpected { .. } | Self::InternalBusFull { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    fn suggestion(&self) -> String {
        match self {
            Self::DaemonNotRunning => {
                "run `trumpet start` to start the daemon".to_owned()
            }
            Self::DaemonAlreadyRunning { pid } => {
                format!("stop the running daemon first: `trumpet stop` (pid {pid})")
            }
            Self::DaemonStaleSocket { path, .. } => {
                format!("remove the stale socket and restart: `rm {path} && trumpet start`")
            }
            Self::DaemonBindFailed { path, .. } => {
                format!("check that '{path}' is writable and no other process holds the socket")
            }
            Self::DaemonShutdownFailed { .. } => {
                "try `trumpet stop --force` or manually kill the daemon process".to_owned()
            }
            Self::ConnectionRefused => {
                "verify the daemon is running with `trumpet status`".to_owned()
            }
            Self::ConnectionTimeout { .. } => {
                "check daemon load with `trumpet status`; try again or restart with `trumpet restart`"
                    .to_owned()
            }
            Self::ConnectionSocketNotFound { path } => {
                format!("start the daemon with `trumpet start`; expected socket at '{path}'")
            }
            Self::AgentNotFound { name } => {
                format!(
                    "list registered agents with `trumpet agent list`; '{name}' was not found"
                )
            }
            Self::AgentAlreadyRegistered { name } => {
                format!("unregister the existing agent first: `trumpet agent remove {name}`")
            }
            Self::AgentInvalidName { .. } => {
                "agent names must be non-empty alphanumeric strings (hyphens allowed)".to_owned()
            }
            Self::ConversationNotFound { id } => {
                format!("list conversations with `trumpet chat list`; '{id}' was not found")
            }
            Self::ConversationNotParticipant { agent, id } => {
                format!("add '{agent}' to conversation '{id}' before sending messages")
            }
            Self::ConversationEmptyParticipants => {
                "provide at least one agent name when creating a conversation".to_owned()
            }
            Self::SkillNotFound { name } => {
                format!("list registered skills with GET /skills; '{name}' was not found")
            }
            Self::SkillNotFoundById { id } => {
                format!("no skill with id '{id}'; list registered skills with GET /skills")
            }
            Self::SkillAlreadyRegistered { name } => {
                format!("deregister the existing skill first or use a different name; '{name}' already exists")
            }
            Self::SkillInvalidName { .. } => {
                "skill names must be 1-64 chars, alphanumeric plus hyphens, underscores, and dots; no leading/trailing hyphen or dot".to_owned()
            }
            Self::SkillInvalidDescription { .. } => {
                "skill description must not be empty".to_owned()
            }
            Self::SkillInvocationFailed { name, reason } => {
                format!("skill '{name}' failed to execute: {reason}")
            }
            Self::SkillNotImplemented { .. } => {
                "skill invocation is not yet implemented".to_owned()
            }
            Self::SkillProviderUnavailable { name } => {
                format!("the agent providing skill '{name}' is not currently connected; check agent status")
            }
            Self::ConfigInvalidToml { path, .. } => {
                format!("fix the TOML syntax error in '{path}'; run `trumpet config validate` for details")
            }
            Self::ConfigMissingDir { path } => {
                format!("create the config directory: `mkdir -p {path}`")
            }
            Self::ConfigPermissionDenied { path } => {
                format!("check file permissions on '{path}' and ensure the process user has read access")
            }
            Self::ConfigValidationFailed { .. } => {
                "review the config schema in the documentation and correct the invalid field".to_owned()
            }
            Self::InternalUnexpected { .. } => {
                "this is a bug — please report it with `trumpet bug-report`".to_owned()
            }
            Self::InternalBusFull { .. } => {
                "the daemon is overloaded; consider reducing agent concurrency or restarting with `trumpet restart`"
                    .to_owned()
            }
        }
    }
}

impl From<crate::config::ConfigError> for Error {
    fn from(err: crate::config::ConfigError) -> Self {
        match err {
            crate::config::ConfigError::InvalidToml { path, source } => Self::ConfigInvalidToml {
                path,
                reason: source.to_string(),
            },
            crate::config::ConfigError::MissingDir(path) => Self::ConfigMissingDir { path },
            crate::config::ConfigError::PermissionDenied(path) => {
                Self::ConfigPermissionDenied { path }
            }
            crate::config::ConfigError::ValidationFailed(reason) => {
                Self::ConfigValidationFailed { reason }
            }
            crate::config::ConfigError::Io { path, source } => Self::InternalUnexpected {
                reason: format!("reading config file '{path}': {source}"),
            },
            crate::config::ConfigError::InvalidEnvVar {
                var,
                value,
                expected,
            } => Self::ConfigValidationFailed {
                reason: format!("env var {var}={value:?}: expected {expected}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One instance of every variant for exhaustive table-driven tests.
    fn all_variants() -> Vec<Error> {
        vec![
            Error::DaemonNotRunning,
            Error::DaemonAlreadyRunning { pid: 1234 },
            Error::DaemonStaleSocket {
                path: "/tmp/trumpet.sock".to_owned(),
                pid: 5678,
            },
            Error::DaemonBindFailed {
                path: "/tmp/trumpet.sock".to_owned(),
                reason: "address in use".to_owned(),
            },
            Error::DaemonShutdownFailed {
                reason: "timed out".to_owned(),
            },
            Error::ConnectionRefused,
            Error::ConnectionTimeout {
                duration: "5s".to_owned(),
            },
            Error::ConnectionSocketNotFound {
                path: "/tmp/trumpet.sock".to_owned(),
            },
            Error::AgentNotFound {
                name: "worker".to_owned(),
            },
            Error::AgentAlreadyRegistered {
                name: "worker".to_owned(),
            },
            Error::AgentInvalidName {
                name: "".to_owned(),
                reason: "empty name".to_owned(),
            },
            Error::ConversationNotFound {
                id: "abc-123".to_owned(),
            },
            Error::ConversationNotParticipant {
                agent: "worker".to_owned(),
                id: "abc-123".to_owned(),
            },
            Error::ConversationEmptyParticipants,
            Error::SkillNotFound {
                name: "scan".to_owned(),
            },
            Error::SkillNotFoundById {
                id: "f47ac10b-58cc-4372-a567-0e02b2c3d479".to_owned(),
            },
            Error::SkillAlreadyRegistered {
                name: "scan".to_owned(),
            },
            Error::SkillInvalidName {
                name: "-bad".to_owned(),
                reason: "leading hyphen".to_owned(),
            },
            Error::SkillInvalidDescription {
                name: "scan".to_owned(),
                reason: "empty".to_owned(),
            },
            Error::SkillInvocationFailed {
                name: "scan".to_owned(),
                reason: "timeout".to_owned(),
            },
            Error::SkillNotImplemented {
                name: "scan".to_owned(),
            },
            Error::SkillProviderUnavailable {
                name: "scan".to_owned(),
            },
            Error::ConfigInvalidToml {
                path: "~/.trumpet/config.toml".to_owned(),
                reason: "unexpected key".to_owned(),
            },
            Error::ConfigMissingDir {
                path: "~/.trumpet".to_owned(),
            },
            Error::ConfigPermissionDenied {
                path: "~/.trumpet/config.toml".to_owned(),
            },
            Error::ConfigValidationFailed {
                reason: "port out of range".to_owned(),
            },
            Error::InternalUnexpected {
                reason: "unreachable branch".to_owned(),
            },
            Error::InternalBusFull { capacity: 1024 },
        ]
    }

    #[test]
    fn test_every_variant_has_non_empty_code() {
        for err in all_variants() {
            let code = err.code();
            assert!(!code.is_empty(), "code() must not be empty for {err:?}");
        }
    }

    #[test]
    fn test_every_variant_has_non_empty_suggestion() {
        for err in all_variants() {
            let suggestion = err.suggestion();
            assert!(
                !suggestion.is_empty(),
                "suggestion() must not be empty for {err:?}"
            );
        }
    }

    /// Validates that every code string matches `SCREAMING_SNAKE_CASE`:
    /// only uppercase ASCII letters, digits, and underscores.
    #[test]
    fn test_codes_are_screaming_snake_case() {
        for err in all_variants() {
            let code = err.code();
            assert!(
                code.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
                "code '{code}' is not SCREAMING_SNAKE_CASE (variant: {err:?})"
            );
        }
    }

    #[test]
    fn test_codes_are_unique() {
        use std::collections::HashSet;
        let codes: Vec<&'static str> = all_variants().iter().map(|e| e.code()).collect();
        let unique: HashSet<_> = codes.iter().copied().collect();
        assert_eq!(
            codes.len(),
            unique.len(),
            "duplicate error codes detected: {codes:?}"
        );
    }

    // ── HTTP status spot-checks ───────────────────────────────────────────────

    #[test]
    fn test_daemon_not_running_is_503() {
        assert_eq!(
            Error::DaemonNotRunning.http_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn test_agent_not_found_is_404() {
        assert_eq!(
            Error::AgentNotFound {
                name: "x".to_owned()
            }
            .http_status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn test_agent_already_registered_is_409() {
        assert_eq!(
            Error::AgentAlreadyRegistered {
                name: "x".to_owned()
            }
            .http_status(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn test_config_invalid_toml_is_422() {
        assert_eq!(
            Error::ConfigInvalidToml {
                path: "p".to_owned(),
                reason: "r".to_owned()
            }
            .http_status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn test_internal_unexpected_is_500() {
        assert_eq!(
            Error::InternalUnexpected {
                reason: "r".to_owned()
            }
            .http_status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
