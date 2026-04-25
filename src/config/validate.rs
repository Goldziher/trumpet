use tracing::trace_span;

use crate::config::{ConfigError, types::Config};

/// Validate semantic constraints on a loaded [`Config`].
///
/// # Errors
///
/// Returns [`ConfigError::ValidationFailed`] with a descriptive message when any
/// constraint is violated.
pub(crate) fn validate(config: &Config) -> Result<(), ConfigError> {
    let _span = trace_span!("config::validate").entered();

    check_ports(config)?;
    check_storage(config)?;
    check_agents(config)?;
    check_code_tools(config)?;
    check_logging(config)?;

    Ok(())
}

fn check_ports(config: &Config) -> Result<(), ConfigError> {
    if config.server.http_port == 0 {
        return Err(ConfigError::ValidationFailed(
            "server.http_port must not be 0".into(),
        ));
    }
    if config.server.grpc_port == 0 {
        return Err(ConfigError::ValidationFailed(
            "server.grpc_port must not be 0".into(),
        ));
    }
    if config.server.http_port == config.server.grpc_port {
        return Err(ConfigError::ValidationFailed(format!(
            "server.http_port and server.grpc_port must differ (both are {})",
            config.server.http_port
        )));
    }
    Ok(())
}

fn check_storage(config: &Config) -> Result<(), ConfigError> {
    if config.storage.snapshot_interval_secs == 0 {
        return Err(ConfigError::ValidationFailed(
            "storage.snapshot_interval_secs must be greater than 0".into(),
        ));
    }
    Ok(())
}

fn check_agents(config: &Config) -> Result<(), ConfigError> {
    if config.agents.heartbeat_interval_secs == 0 {
        return Err(ConfigError::ValidationFailed(
            "agents.heartbeat_interval_secs must be greater than 0".into(),
        ));
    }
    if config.agents.timeout_secs == 0 {
        return Err(ConfigError::ValidationFailed(
            "agents.timeout_secs must be greater than 0".into(),
        ));
    }
    if config.agents.timeout_secs <= config.agents.heartbeat_interval_secs {
        return Err(ConfigError::ValidationFailed(format!(
            "agents.timeout_secs ({}) must be greater than agents.heartbeat_interval_secs ({})",
            config.agents.timeout_secs, config.agents.heartbeat_interval_secs
        )));
    }
    Ok(())
}

fn check_code_tools(config: &Config) -> Result<(), ConfigError> {
    if config.code_tools.max_file_size_bytes == 0 {
        return Err(ConfigError::ValidationFailed(
            "code_tools.max_file_size_bytes must be greater than 0".into(),
        ));
    }
    if config.code_tools.max_files_per_page == 0 {
        return Err(ConfigError::ValidationFailed(
            "code_tools.max_files_per_page must be greater than 0".into(),
        ));
    }
    if config.code_tools.max_search_matches == 0 {
        return Err(ConfigError::ValidationFailed(
            "code_tools.max_search_matches must be greater than 0".into(),
        ));
    }
    Ok(())
}

fn check_logging(config: &Config) -> Result<(), ConfigError> {
    // Validate that the level string parses as a tracing EnvFilter directive.
    config
        .logging
        .level
        .parse::<tracing_subscriber::EnvFilter>()
        .map_err(|e| {
            ConfigError::ValidationFailed(format!(
                "logging.level {:?} is not a valid tracing filter: {e}",
                config.logging.level
            ))
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::types::{AgentsConfig, CodeToolsConfig, ServerConfig, StorageConfig};

    fn valid_config() -> Config {
        Config::default()
    }

    #[test]
    fn validate_accepts_valid_defaults() {
        let config = valid_config();
        assert!(
            validate(&config).is_ok(),
            "Config::default() must pass validation"
        );
    }

    #[test]
    fn validate_rejects_duplicate_ports() {
        let mut config = valid_config();
        config.server.http_port = 7600;
        config.server.grpc_port = 7600;
        let err = validate(&config).expect_err("duplicate ports must fail validation");
        assert!(
            err.to_string().contains("must differ"),
            "error message should mention ports must differ, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_http_port() {
        let mut config = valid_config();
        config.server.http_port = 0;
        let err = validate(&config).expect_err("port 0 must fail validation");
        assert!(
            err.to_string().contains("http_port"),
            "error should reference http_port, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_grpc_port() {
        let mut config = valid_config();
        config.server.grpc_port = 0;
        let err = validate(&config).expect_err("grpc port 0 must fail validation");
        assert!(
            err.to_string().contains("grpc_port"),
            "error should reference grpc_port, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_snapshot_interval() {
        let mut config = valid_config();
        config.storage.snapshot_interval_secs = 0;
        let err = validate(&config).expect_err("zero snapshot interval must fail");
        assert!(
            err.to_string().contains("snapshot_interval_secs"),
            "error should reference snapshot_interval_secs, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_heartbeat_interval() {
        let mut config = valid_config();
        config.agents.heartbeat_interval_secs = 0;
        let err = validate(&config).expect_err("zero heartbeat interval must fail");
        assert!(
            err.to_string().contains("heartbeat_interval_secs"),
            "error should reference heartbeat_interval_secs, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_timeout_not_greater_than_heartbeat() {
        let mut config = valid_config();
        config.agents.heartbeat_interval_secs = 60;
        config.agents.timeout_secs = 60;
        let err = validate(&config).expect_err("timeout == heartbeat must fail");
        assert!(
            err.to_string().contains("timeout_secs"),
            "error should reference timeout_secs, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_zero_max_file_size() {
        let mut config = valid_config();
        config.code_tools.max_file_size_bytes = 0;
        let err = validate(&config).expect_err("zero max_file_size_bytes must fail");
        assert!(
            err.to_string().contains("max_file_size_bytes"),
            "error should reference max_file_size_bytes, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_invalid_log_level() {
        let mut config = valid_config();
        config.logging.level = "not_a_real_level[[[".into();
        let err = validate(&config).expect_err("invalid log level must fail");
        assert!(
            err.to_string().contains("logging.level"),
            "error should reference logging.level, got: {err}"
        );
    }

    #[test]
    fn validate_accepts_debug_log_level() {
        let mut config = valid_config();
        config.logging.level = "debug".into();
        assert!(
            validate(&config).is_ok(),
            "debug log level must be accepted"
        );
    }

    // Verify default structs are consistent with validate expectations.
    #[test]
    fn default_server_config_passes_validation() {
        let config = Config {
            server: ServerConfig::default(),
            ..Config::default()
        };
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn default_storage_config_passes_validation() {
        let config = Config {
            storage: StorageConfig::default(),
            ..Config::default()
        };
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn default_agents_config_passes_validation() {
        let config = Config {
            agents: AgentsConfig::default(),
            ..Config::default()
        };
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn default_code_tools_config_passes_validation() {
        let config = Config {
            code_tools: CodeToolsConfig::default(),
            ..Config::default()
        };
        assert!(validate(&config).is_ok());
    }
}
