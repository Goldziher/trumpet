//! `trumpet serve` subcommand — starts the daemon in the foreground.

use crate::config::{Config, LogFormat};
use crate::error::Result;

/// Load config, apply logging settings, and start the server.
///
/// Blocks until a shutdown signal (Ctrl-C) is received.
pub async fn run_serve() -> Result<()> {
    let config = Config::load()?;
    init_logging(&config);
    crate::server::serve(&config).await
}

/// (Re-)initialize tracing with the loaded config's level and format.
fn init_logging(config: &Config) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.logging.level));

    match config.logging.format {
        LogFormat::Json => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .init();
        }
        LogFormat::Text => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
}
