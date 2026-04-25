use std::path::{Path, PathBuf};

use toml::Value;
use tracing::debug;

use crate::config::{ConfigError, types::Config};

/// Load and merge configuration from files and environment variables.
///
/// Layering order (each layer overrides the previous):
/// 1. Compiled defaults ([`Config::default`])
/// 2. User config (`~/.trumpet/config.toml`) — skipped if absent
/// 3. Project config (`./trumpet.toml`) — skipped if absent
/// 4. Environment variables (`TRUMPET_*`)
///
/// After merging, `$HOME`-relative paths are resolved.
///
/// # Errors
///
/// Returns [`ConfigError::InvalidToml`] if a present config file cannot be parsed.
pub(crate) fn load_layered() -> Result<Config, ConfigError> {
    // Start from compiled defaults serialized to a TOML Value table so we can
    // do deep-merge on top of it.
    let mut base = to_value_table(&Config::default());

    // User config: ~/.trumpet/config.toml
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    let user_path = home.join(".trumpet").join("config.toml");
    if user_path.exists() {
        debug!(path = %user_path.display(), "loading user config");
        let table = read_toml_file(&user_path)?;
        merge_tables(&mut base, table);
    }

    // Project config: ./trumpet.toml
    let project_path = PathBuf::from("trumpet.toml");
    if project_path.exists() {
        debug!(path = %project_path.display(), "loading project config");
        let table = read_toml_file(&project_path)?;
        merge_tables(&mut base, table);
    }

    // Serialise the merged table back to a TOML string then parse into Config.
    // This round-trip is reliable and works with all toml crate versions.
    let merged_toml = toml::to_string(&Value::Table(base))
        .expect("merged Config table is always serialisable to TOML");
    let mut config: Config =
        toml::from_str(&merged_toml).map_err(|e| ConfigError::InvalidToml {
            path: "<merged>".into(),
            source: e,
        })?;

    // Apply environment variable overrides.
    apply_env_vars(&mut config)?;

    // Resolve $HOME-relative paths.
    resolve_paths(&mut config, &home);

    Ok(config)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Serialize a [`Config`] to a [`toml::value::Table`].
///
/// # Panics
///
/// Panics if `Config` cannot be serialized — this is a programming error since
/// `Config` is always a valid TOML structure.
fn to_value_table(config: &Config) -> toml::value::Table {
    let value = toml::Value::try_from(config).expect("Config is always serializable to TOML");
    match value {
        Value::Table(t) => t,
        _ => unreachable!("Config serializes to a TOML table"),
    }
}

/// Read and parse a TOML file into a [`toml::value::Table`].
///
/// Returns an error if the file cannot be read or contains invalid TOML.
/// A valid TOML file that is not a table document is treated as an error.
fn read_toml_file(path: &Path) -> Result<toml::value::Table, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            ConfigError::PermissionDenied(path.display().to_string())
        }
        _ => ConfigError::Io {
            path: path.display().to_string(),
            source: e,
        },
    })?;

    // Every syntactically valid TOML document is a key-value table at the top
    // level (per the TOML spec). Parsing directly into `toml::value::Table`
    // via serde gives a typed `toml::de::Error` on both parse and type errors.
    toml::from_str::<toml::value::Table>(&content).map_err(|e| ConfigError::InvalidToml {
        path: path.display().to_string(),
        source: e,
    })
}

/// Recursively merge `overlay` on top of `base`.
///
/// Scalar values in `overlay` replace those in `base`. Tables are merged
/// recursively. Arrays in `overlay` replace those in `base`.
fn merge_tables(base: &mut toml::value::Table, overlay: toml::value::Table) {
    for (key, val) in overlay {
        match (base.get_mut(&key), val) {
            (Some(Value::Table(base_table)), Value::Table(overlay_table)) => {
                merge_tables(base_table, overlay_table);
            }
            (_, val) => {
                base.insert(key, val);
            }
        }
    }
}

/// Apply `TRUMPET_*` environment variable overrides to `config`.
///
/// Supported variables and their mapping:
/// - `TRUMPET_SERVER_HOST` → `config.server.host`
/// - `TRUMPET_SERVER_HTTP_PORT` → `config.server.http_port`
/// - `TRUMPET_SERVER_GRPC_PORT` → `config.server.grpc_port`
/// - `TRUMPET_STORAGE_BACKEND` → `config.storage.backend`
/// - `TRUMPET_STORAGE_PATH` → `config.storage.path`
/// - `TRUMPET_LOGGING_LEVEL` → `config.logging.level`
/// - `TRUMPET_LOGGING_FORMAT` → `config.logging.format` (`"text"` or `"json"`)
fn apply_env_vars(config: &mut Config) -> Result<(), ConfigError> {
    if let Ok(val) = std::env::var("TRUMPET_SERVER_HOST") {
        config.server.host = val;
    }
    if let Ok(val) = std::env::var("TRUMPET_SERVER_HTTP_PORT") {
        config.server.http_port = val.parse::<u16>().map_err(|_| ConfigError::InvalidEnvVar {
            var: "TRUMPET_SERVER_HTTP_PORT".into(),
            value: val,
            expected: "u16 port number".into(),
        })?;
    }
    if let Ok(val) = std::env::var("TRUMPET_SERVER_GRPC_PORT") {
        config.server.grpc_port = val.parse::<u16>().map_err(|_| ConfigError::InvalidEnvVar {
            var: "TRUMPET_SERVER_GRPC_PORT".into(),
            value: val,
            expected: "u16 port number".into(),
        })?;
    }
    if let Ok(val) = std::env::var("TRUMPET_STORAGE_BACKEND") {
        config.storage.backend = val;
    }
    if let Ok(val) = std::env::var("TRUMPET_STORAGE_PATH") {
        config.storage.path = PathBuf::from(val);
    }
    if let Ok(val) = std::env::var("TRUMPET_LOGGING_LEVEL") {
        config.logging.level = val;
    }
    if let Ok(val) = std::env::var("TRUMPET_LOGGING_FORMAT") {
        match val.as_str() {
            "json" => config.logging.format = crate::config::types::LogFormat::Json,
            "text" => config.logging.format = crate::config::types::LogFormat::Text,
            _ => {
                return Err(ConfigError::InvalidEnvVar {
                    var: "TRUMPET_LOGGING_FORMAT".into(),
                    value: val,
                    expected: "\"text\" or \"json\"".into(),
                });
            }
        }
    }
    Ok(())
}

/// Resolve placeholder paths that still hold compiled defaults.
///
/// Only overwrites a path if it matches the static `Default::default()` value,
/// preserving any user-configured paths from config files or env vars.
fn resolve_paths(config: &mut Config, home: &Path) {
    let base = home.join(".trumpet");
    let defaults = crate::config::types::DaemonConfig::default();

    if config.daemon.socket_path == defaults.socket_path {
        config.daemon.socket_path = base.join("trumpet.sock");
    }
    if config.daemon.pid_file == defaults.pid_file {
        config.daemon.pid_file = base.join("trumpet.pid");
    }

    let storage_default = crate::config::types::StorageConfig::default();
    if config.storage.path == storage_default.path {
        config.storage.path = base.join("state");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use serial_test::serial;

    use super::*;
    use crate::config::test_helpers::{make_trumpet_dir, with_cwd, with_env, with_home};
    use crate::config::types::{LogFormat, McpTransport};

    #[test]
    #[serial]
    fn load_with_user_config_file() {
        let home = TempDir::new().unwrap();
        let trumpet_dir = make_trumpet_dir(&home);

        let toml = r#"
[server]
http_port = 8080
"#;
        fs::write(trumpet_dir.join("config.toml"), toml).unwrap();

        with_home(&home, || {
            let config = load_layered().expect("load_layered must succeed");
            assert_eq!(
                config.server.http_port, 8080,
                "user config http_port override must be applied"
            );
            // Non-overridden fields retain defaults.
            assert_eq!(
                config.server.grpc_port, 7601,
                "grpc_port must remain default when not overridden"
            );
        });
    }

    // ------------------------------------------------------------------
    // load_with_project_config_overrides_user
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn load_with_project_config_overrides_user() {
        let home = TempDir::new().unwrap();
        let trumpet_dir = make_trumpet_dir(&home);

        // User config sets http_port = 8080.
        fs::write(
            trumpet_dir.join("config.toml"),
            "[server]\nhttp_port = 8080\n",
        )
        .unwrap();

        // Project config sets http_port = 9090 — must win.
        let project_dir = TempDir::new().unwrap();
        fs::write(
            project_dir.path().join("trumpet.toml"),
            "[server]\nhttp_port = 9090\n",
        )
        .unwrap();

        // Change working directory to project_dir so ./trumpet.toml is found.
        with_cwd(project_dir.path(), || {
            with_home(&home, || {
                let config = load_layered().expect("load_layered must succeed");
                assert_eq!(
                    config.server.http_port, 9090,
                    "project config must override user config"
                );
            });
        });
    }

    // ------------------------------------------------------------------
    // env_var_overrides_file
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn env_var_overrides_file() {
        let home = TempDir::new().unwrap();
        let trumpet_dir = make_trumpet_dir(&home);

        fs::write(
            trumpet_dir.join("config.toml"),
            "[server]\nhttp_port = 8080\n",
        )
        .unwrap();

        with_home(&home, || {
            with_env("TRUMPET_SERVER_HTTP_PORT", "9999", || {
                let config = load_layered().expect("load_layered must succeed");
                assert_eq!(
                    config.server.http_port, 9999,
                    "env var TRUMPET_SERVER_HTTP_PORT must override file value"
                );
            });
        });
    }

    // ------------------------------------------------------------------
    // partial_toml_file_fills_defaults
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn partial_toml_file_fills_defaults() {
        let home = TempDir::new().unwrap();
        let trumpet_dir = make_trumpet_dir(&home);

        // Only the [daemon] section is present.
        fs::write(trumpet_dir.join("config.toml"), "[daemon]\n").unwrap();

        with_home(&home, || {
            let config = load_layered().expect("load_layered must succeed with partial file");
            // All other sections should have their defaults.
            assert_eq!(
                config.server.http_port, 7600,
                "http_port must default to 7600 when not in file"
            );
            assert_eq!(
                config.mcp.transport,
                McpTransport::Stdio,
                "mcp.transport must default to Stdio"
            );
            assert_eq!(
                config.logging.level, "info",
                "logging.level must default to info"
            );
            assert_eq!(
                config.logging.format,
                LogFormat::Text,
                "logging.format must default to text"
            );
            assert_eq!(
                config.agents.heartbeat_interval_secs, 30,
                "agents.heartbeat_interval_secs must default to 30"
            );
            assert_eq!(
                config.code_tools.max_file_size_bytes, 1_048_576,
                "code_tools.max_file_size_bytes must default to 1 MiB"
            );
        });
    }

    // ------------------------------------------------------------------
    // env var coverage for other keys
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn env_var_trumpet_server_host_is_applied() {
        let home = TempDir::new().unwrap();
        make_trumpet_dir(&home);

        with_home(&home, || {
            with_env("TRUMPET_SERVER_HOST", "0.0.0.0", || {
                let config = load_layered().expect("load_layered must succeed");
                assert_eq!(config.server.host, "0.0.0.0");
            });
        });
    }

    #[test]
    #[serial]
    fn env_var_trumpet_logging_format_json_is_applied() {
        let home = TempDir::new().unwrap();
        make_trumpet_dir(&home);

        with_home(&home, || {
            with_env("TRUMPET_LOGGING_FORMAT", "json", || {
                let config = load_layered().expect("load_layered must succeed");
                assert_eq!(config.logging.format, LogFormat::Json);
            });
        });
    }

    // ------------------------------------------------------------------
    // Merge helper unit tests
    // ------------------------------------------------------------------

    #[test]
    fn merge_tables_scalar_override() {
        let mut base: toml::value::Table = toml::from_str("a = 1\nb = 2").unwrap();
        let overlay: toml::value::Table = toml::from_str("b = 99").unwrap();
        merge_tables(&mut base, overlay);
        assert_eq!(base["a"], Value::Integer(1));
        assert_eq!(base["b"], Value::Integer(99));
    }

    #[test]
    fn merge_tables_nested_partial_override() {
        let mut base: toml::value::Table =
            toml::from_str("[server]\nhttp_port = 7600\ngrpc_port = 7601").unwrap();
        let overlay: toml::value::Table = toml::from_str("[server]\nhttp_port = 8080").unwrap();
        merge_tables(&mut base, overlay);
        let server = base["server"].as_table().unwrap();
        assert_eq!(server["http_port"], Value::Integer(8080));
        assert_eq!(server["grpc_port"], Value::Integer(7601));
    }

    // ------------------------------------------------------------------
    // Missing file is silently skipped
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn missing_user_config_file_is_silently_skipped() {
        let home = TempDir::new().unwrap();
        // Do NOT create ~/.trumpet/config.toml.
        with_home(&home, || {
            let config = load_layered().expect("load_layered must succeed without user config");
            assert_eq!(config.server.http_port, 7600, "defaults must apply");
        });
    }

    // ------------------------------------------------------------------
    // Invalid TOML returns ConfigError::InvalidToml
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn invalid_toml_in_user_config_returns_error() {
        let home = TempDir::new().unwrap();
        let trumpet_dir = make_trumpet_dir(&home);
        fs::write(trumpet_dir.join("config.toml"), "not valid toml ][[[").unwrap();

        with_home(&home, || {
            let err = load_layered().expect_err("invalid TOML must return an error");
            assert!(
                matches!(err, ConfigError::InvalidToml { .. }),
                "expected InvalidToml, got: {err}"
            );
        });
    }

    // ------------------------------------------------------------------
    // storage.path is resolved relative to HOME
    // ------------------------------------------------------------------

    #[test]
    #[serial]
    fn storage_path_resolved_under_home() {
        let home = TempDir::new().unwrap();
        make_trumpet_dir(&home);

        with_home(&home, || {
            let config = load_layered().expect("load_layered must succeed");
            assert!(
                config.storage.path.starts_with(home.path()),
                "storage.path must be under HOME, got: {:?}",
                config.storage.path
            );
            assert!(
                config.storage.path.ends_with("state"),
                "storage.path must end with 'state', got: {:?}",
                config.storage.path
            );
        });
    }
}
