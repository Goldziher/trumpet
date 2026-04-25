use thiserror::Error;

/// Errors that can occur while loading or validating configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The config file contains invalid TOML.
    #[error("invalid TOML in {path}: {source}")]
    InvalidToml {
        /// Path to the offending config file.
        path: String,
        /// Underlying parse error.
        #[source]
        source: toml::de::Error,
    },

    /// A required directory does not exist or could not be created.
    #[error("missing directory: {0}")]
    MissingDir(String),

    /// An I/O error occurred while reading a config file.
    #[error("reading config file '{path}': {source}")]
    Io {
        /// Path to the offending config file.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The process lacks permission to access a path.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// An environment variable was set but contained an invalid value.
    #[error("invalid value for env var {var}: expected {expected}, got '{value}'")]
    InvalidEnvVar {
        /// Name of the environment variable.
        var: String,
        /// The invalid value.
        value: String,
        /// What was expected.
        expected: String,
    },

    /// Configuration values failed semantic validation.
    #[error("validation failed: {0}")]
    ValidationFailed(String),
}
