//! HTTP server for the Trumpet daemon.
//!
//! The server listens on a Unix domain socket and exposes a JSON REST API
//! for agent management, conversation management, and a Server-Sent Events
//! stream for real-time event delivery.

mod error;
mod routes;
mod state;

pub use state::AppState;

use tokio::net::UnixListener;
use tracing::info;

use crate::config::Config;
use crate::error::Result;

/// Start the Trumpet daemon HTTP server.
///
/// Binds a Unix domain socket at the path specified in `config.daemon.socket_path`,
/// serves the REST API, and blocks until a shutdown signal (Ctrl-C) is received.
/// The socket file is removed on both clean and error shutdown paths.
///
/// # Errors
///
/// Returns an error if the socket cannot be bound or if axum fails to serve.
pub async fn serve(config: &Config) -> Result<()> {
    let socket_path = &config.daemon.socket_path;

    // Remove stale socket left by a previous run.
    if socket_path.exists() {
        tokio::fs::remove_file(socket_path).await.map_err(|e| {
            crate::error::Error::DaemonBindFailed {
                path: socket_path.display().to_string(),
                reason: format!("failed to remove stale socket: {e}"),
            }
        })?;
    }

    // Ensure the parent directory exists.
    if let Some(parent) = socket_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|_| {
            crate::error::Error::ConfigMissingDir {
                path: parent.display().to_string(),
            }
        })?;
    }

    let listener =
        UnixListener::bind(socket_path).map_err(|e| crate::error::Error::DaemonBindFailed {
            path: socket_path.display().to_string(),
            reason: e.to_string(),
        })?;

    info!("trumpet daemon listening on {}", socket_path.display());

    let state = AppState::new(config.clone());
    let app = routes::router(state);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| crate::error::Error::InternalUnexpected {
            reason: e.to_string(),
        })?;

    // Best-effort socket cleanup after graceful shutdown.
    let _ = tokio::fs::remove_file(socket_path).await;
    info!("trumpet daemon stopped");

    Ok(())
}

/// Resolves when Ctrl-C is received.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    info!("shutdown signal received");
}
