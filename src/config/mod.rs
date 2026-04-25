mod error;
mod loader;
pub mod types;
mod validate;

#[cfg(test)]
mod test_helpers;

pub use error::ConfigError;
pub use types::{
    AgentsConfig, CodeToolsConfig, Config, DaemonConfig, LogFormat, LoggingConfig, McpConfig,
    McpTransport, ServerConfig, StorageConfig,
};

impl Config {
    /// Load configuration, applying the layering order:
    /// compiled defaults -> user config (`~/.trumpet/config.toml`) ->
    /// project config (`./trumpet.toml`) -> env vars (`TRUMPET_*`).
    ///
    /// Paths that are `$HOME`-relative are resolved before returning.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a config file is present but cannot be parsed
    /// or the resulting config fails semantic validation.
    pub fn load() -> Result<Self, ConfigError> {
        let config = loader::load_layered()?;
        validate::validate(&config)?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;

    use super::*;
    use crate::config::test_helpers::{make_trumpet_dir, with_env, with_home};
    use std::fs;
    use tempfile::TempDir;

    // ── defaults ─────────────────────────────────────────────────────────────

    #[test]
    fn default_socket_path_is_placeholder() {
        let config = Config::default();
        assert_eq!(
            config.daemon.socket_path,
            std::path::PathBuf::from("/tmp/trumpet/trumpet.sock"),
        );
    }

    #[test]
    fn default_pid_file_is_placeholder() {
        let config = Config::default();
        assert_eq!(
            config.daemon.pid_file,
            std::path::PathBuf::from("/tmp/trumpet/trumpet.pid"),
        );
    }

    #[test]
    fn toml_round_trip_preserves_equality() {
        let original = Config::default();
        let serialized = toml::to_string(&original).expect("serialization must succeed");
        let deserialized: Config =
            toml::from_str(&serialized).expect("deserialization must succeed");
        assert_eq!(original, deserialized);
    }

    // ── Config::load integration ─────────────────────────────────────────────

    #[test]
    #[serial]
    fn load_resolves_paths_under_home() {
        let home = TempDir::new().unwrap();
        make_trumpet_dir(&home);
        with_home(&home, || {
            let config = Config::load().expect("Config::load must succeed");
            assert!(config.daemon.socket_path.ends_with("trumpet.sock"));
            assert!(
                config
                    .daemon
                    .socket_path
                    .to_string_lossy()
                    .contains(".trumpet"),
            );
            assert!(config.daemon.pid_file.ends_with("trumpet.pid"));
        });
    }

    #[test]
    #[serial]
    fn load_end_to_end_with_file_env_and_validation() {
        let home = TempDir::new().unwrap();
        let trumpet_dir = make_trumpet_dir(&home);
        fs::write(
            trumpet_dir.join("config.toml"),
            "[server]\nhttp_port = 8080\n",
        )
        .unwrap();
        with_home(&home, || {
            with_env("TRUMPET_SERVER_HTTP_PORT", "9999", || {
                let config = Config::load().expect("Config::load must succeed");
                assert_eq!(config.server.http_port, 9999, "env var must win over file");
            });
        });
    }
}
