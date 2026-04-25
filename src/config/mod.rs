mod error;
mod types;

pub use error::ConfigError;
pub use types::{Config, DaemonConfig, LogFormat, LoggingConfig};

impl Config {
    /// Load configuration, applying the layering order:
    /// compiled defaults → user config → project config → env vars → CLI flags.
    ///
    /// For milestone 1 this returns compiled defaults only.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if a config file is present but cannot be parsed
    /// or fails validation.
    pub fn load() -> Result<Self, ConfigError> {
        let mut config = Self::default();
        let base = std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
            .join(".trumpet");
        config.daemon.socket_path = base.join("trumpet.sock");
        config.daemon.pid_file = base.join("trumpet.pid");
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_socket_path_is_placeholder() {
        let config = Config::default();
        assert_eq!(
            config.daemon.socket_path,
            std::path::PathBuf::from("/tmp/trumpet/trumpet.sock"),
            "default() must return static placeholder socket path"
        );
    }

    #[test]
    fn default_pid_file_is_placeholder() {
        let config = Config::default();
        assert_eq!(
            config.daemon.pid_file,
            std::path::PathBuf::from("/tmp/trumpet/trumpet.pid"),
            "default() must return static placeholder pid path"
        );
    }

    #[test]
    fn toml_round_trip_preserves_equality() {
        let original = Config::default();
        let serialized = toml::to_string(&original).expect("serialization must succeed");
        let deserialized: Config =
            toml::from_str(&serialized).expect("deserialization must succeed");
        assert_eq!(
            original, deserialized,
            "round-trip produced a different Config"
        );
    }

    #[test]
    fn load_resolves_paths_under_home() {
        let config = Config::load().expect("Config::load must succeed");
        assert!(
            config.daemon.socket_path.ends_with("trumpet.sock"),
            "load() socket_path must end with 'trumpet.sock', got {:?}",
            config.daemon.socket_path,
        );
        assert!(
            config
                .daemon
                .socket_path
                .to_string_lossy()
                .contains(".trumpet"),
            "load() socket_path must be under .trumpet dir",
        );
        assert!(
            config.daemon.pid_file.ends_with("trumpet.pid"),
            "load() pid_file must end with 'trumpet.pid'",
        );
    }
}
