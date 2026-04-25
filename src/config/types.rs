use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Top-level configuration, composed of section structs.
///
/// Loaded via [`Config::load`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Daemon process settings.
    #[serde(default)]
    pub daemon: DaemonConfig,
    /// HTTP and gRPC server settings.
    #[serde(default)]
    pub server: ServerConfig,
    /// MCP server transport settings.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Persistent state storage settings.
    #[serde(default)]
    pub storage: StorageConfig,
    /// Logging settings.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Default settings for connected agents.
    #[serde(default)]
    pub agents: AgentsConfig,
    /// Tree-sitter code tool limits.
    #[serde(default)]
    pub code_tools: CodeToolsConfig,
    /// Local-process authentication settings.
    #[serde(default)]
    pub security: SecurityConfig,
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

/// HTTP and gRPC server settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Host address the server binds to.
    pub host: String,
    /// Port for the HTTP server.
    pub http_port: u16,
    /// Port for the gRPC server.
    pub grpc_port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            http_port: 7600,
            grpc_port: 7601,
        }
    }
}

/// MCP server transport settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpConfig {
    /// Transport mechanism for the MCP server.
    pub transport: McpTransport,
    /// Whether the MCP server is enabled. Disable when running as a
    /// background daemon where stdin is not available.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            transport: McpTransport::Stdio,
            enabled: true,
        }
    }
}

/// Transport mechanism for the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// Standard I/O transport.
    Stdio,
    /// HTTP transport.
    Http,
}

/// Persistent state storage settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Storage backend identifier (e.g. `"fs"`).
    pub backend: String,
    /// Path to the storage root directory.
    pub path: PathBuf,
    /// Interval in seconds between automatic state snapshots.
    pub snapshot_interval_secs: u64,
}

impl Default for StorageConfig {
    /// Returns static placeholder paths.
    ///
    /// The real path is resolved from `$HOME` in [`Config::load`].
    fn default() -> Self {
        Self {
            backend: "fs".into(),
            path: PathBuf::from("/tmp/trumpet/state"),
            snapshot_interval_secs: 60,
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

/// Default settings for connected agents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentsConfig {
    /// Interval in seconds between agent heartbeat checks.
    pub heartbeat_interval_secs: u64,
    /// Seconds before an unresponsive agent is considered timed out.
    pub timeout_secs: u64,
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval_secs: 30,
            timeout_secs: 300,
        }
    }
}

/// Local-process authentication settings.
///
/// REST/WebSocket access is gated by a Unix-socket peer-credential check
/// (the connecting process's UID must match the daemon's). gRPC access is
/// gated by a shared bearer token written to [`SecurityConfig::auth_token_path`]
/// at daemon startup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Path to the bearer-token file. Auto-generated with `0600` permissions
    /// at daemon startup if missing. Defaults to `~/.trumpet/auth.token` in
    /// the loader after `$HOME` resolution.
    pub auth_token_path: PathBuf,
    /// When `false`, peer-credential and bearer-token checks are skipped.
    /// Set this only for local development or test daemons; production usage
    /// must keep it `true`.
    #[serde(default = "default_true")]
    pub require_auth: bool,
}

impl Default for SecurityConfig {
    /// Placeholder path; resolved relative to `$HOME` in [`Config::load`].
    fn default() -> Self {
        Self {
            auth_token_path: PathBuf::from("/tmp/trumpet/auth.token"),
            require_auth: true,
        }
    }
}

/// Tree-sitter code tool limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeToolsConfig {
    /// Maximum file size in bytes the code tools will process.
    pub max_file_size_bytes: u64,
    /// Maximum number of files returned in a single page.
    pub max_files_per_page: u32,
    /// Maximum number of search matches returned.
    pub max_search_matches: u32,
    /// Sandbox root for filesystem access. All paths supplied to code tools
    /// must canonicalize to a descendant of this directory.
    ///
    /// When `None`, the loader populates it with the daemon's current working
    /// directory at startup. Callers that construct [`CodeToolsConfig`]
    /// programmatically must set this explicitly or accept the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<PathBuf>,
    /// Directory names to skip when walking a workspace tree. Matched
    /// case-sensitively against the directory's file name. Hidden directories
    /// (names starting with `.`) are always skipped in addition.
    #[serde(default = "default_walk_skip_dirs")]
    pub walk_skip_dirs: Vec<String>,
}

/// Default deny list for [`CodeToolsConfig::walk_skip_dirs`].
fn default_walk_skip_dirs() -> Vec<String> {
    vec![
        "target".to_owned(),
        "node_modules".to_owned(),
        "vendor".to_owned(),
        "dist".to_owned(),
        "build".to_owned(),
    ]
}

impl Default for CodeToolsConfig {
    fn default() -> Self {
        Self {
            max_file_size_bytes: 1_048_576,
            max_files_per_page: 500,
            max_search_matches: 1000,
            workspace_root: None,
            walk_skip_dirs: default_walk_skip_dirs(),
        }
    }
}
