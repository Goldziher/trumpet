//! `trumpet serve` subcommand — starts the daemon in the foreground.

use crate::config::Config;
use crate::error::Result;

/// Load config and start the HTTP server on the configured Unix socket.
///
/// Blocks until a shutdown signal (Ctrl-C) is received.
///
/// # Errors
///
/// Returns an error if config is invalid or the server fails to bind / serve.
pub async fn run_serve() -> Result<()> {
    let config = Config::load()?;
    crate::server::serve(&config).await
}
