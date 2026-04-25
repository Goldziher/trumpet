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

    /// The process lacks permission to access a path.
    #[error("permission denied: {0}")]
    PermissionDenied(String),

    /// Configuration values failed semantic validation.
    #[error("validation failed: {0}")]
    ValidationFailed(String),
}
