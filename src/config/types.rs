use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level configuration, composed of section structs.
///
/// Loaded via [`Config::load`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Daemon process settings.
    pub daemon: DaemonConfig,
    /// Logging settings.
    pub logging: LoggingConfig,
}

/// Daemon process settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Unix socket path the daemon listens on.
    pub socket_path: PathBuf,
    /// Path to the PID file written on startup.
    pub pid_file: PathBuf,
}

impl Default for DaemonConfig {
    /// Returns static placeholder paths.
    ///
    /// The real paths are resolved from `$HOME` in [`Config::load`].
    fn default() -> Self {
        Self {
            socket_path: PathBuf::from("/tmp/trumpet/trumpet.sock"),
            pid_file: PathBuf::from("/tmp/trumpet/trumpet.pid"),
        }
    }
}

/// Logging configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Tracing level filter string, e.g. `"info"` or `"debug"`.
    pub level: String,
    /// Structured log output format.
    pub format: LogFormat,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".into(),
            format: LogFormat::Text,
        }
    }
}

/// Output format for structured log events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Human-readable plain-text output.
    Text,
    /// Machine-readable JSON output.
    Json,
}
