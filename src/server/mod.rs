//! HTTP server for the Trumpet daemon.
//!
//! The server listens on a Unix domain socket and exposes a JSON REST API,
//! SSE, WebSocket, gRPC (A2A), and optionally an MCP stdio server.

mod error;
mod routes;
mod state;
pub(crate) mod ws;

pub use state::AppState;

use std::sync::Arc;

use tokio::net::UnixListener;
use tracing::info;

use crate::config::Config;
use crate::core::code_tools::CodeTools;
use crate::core::types::SkillProvider;
use crate::error::Result;
use crate::grpc::proto::a2a_service_server::A2aServiceServer;
use crate::grpc::service::NexusA2aService;
use crate::state::StateManager;

/// Start the Trumpet daemon.
///
/// Binds a Unix domain socket for HTTP, starts gRPC on the configured TCP
/// port, restores persisted state, registers built-in skills, spawns a
/// periodic snapshot timer, and blocks until Ctrl-C.
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

    // ── State restore ────────────────────────────────────────────────────────
    let state_manager = Arc::new(StateManager::new(&config.storage).await?);
    let state = AppState::new(config.clone());

    if let Some(snapshot) = state_manager.load_snapshot().await? {
        info!("restoring state from snapshot");
        state.restore_from_snapshot(snapshot).await;
    }

    // ── Register built-in code tools as skills ───────────────────────────────
    register_code_tools(&state, config).await;

    // ── gRPC server ──────────────────────────────────────────────────────────
    let grpc_addr = format!("{}:{}", config.server.host, config.server.grpc_port);
    let grpc_service = NexusA2aService::new(state.clone());
    let grpc_handle = tokio::spawn(async move {
        info!(addr = %grpc_addr, "gRPC server starting");
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(A2aServiceServer::new(grpc_service))
            .serve(grpc_addr.parse().unwrap())
            .await
        {
            tracing::error!(error = %e, "gRPC server failed");
        }
    });

    // ── Periodic snapshot timer ──────────────────────────────────────────────
    let snapshot_interval = config.storage.snapshot_interval_secs;
    let snapshot_state = state.clone();
    let snapshot_mgr = Arc::clone(&state_manager);
    let snapshot_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(snapshot_interval));
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;
            let snap = snapshot_state.to_snapshot().await;
            if let Err(e) = snapshot_mgr.save_snapshot(&snap).await {
                tracing::error!(error = %e, "periodic snapshot failed");
            } else {
                tracing::debug!("periodic snapshot saved");
            }
        }
    });

    // ── HTTP server ──────────────────────────────────────────────────────────
    let app = routes::router(state.clone());

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| crate::error::Error::InternalUnexpected {
            reason: e.to_string(),
        })?;

    // ── Shutdown ─────────────────────────────────────────────────────────────
    snapshot_handle.abort();
    grpc_handle.abort();

    info!("saving state snapshot before shutdown");
    let snapshot = state.to_snapshot().await;
    if let Err(e) = state_manager.save_snapshot(&snapshot).await {
        tracing::error!(error = %e, "failed to save shutdown snapshot");
    }

    let _ = tokio::fs::remove_file(socket_path).await;
    info!("trumpet daemon stopped");

    Ok(())
}

/// Register built-in code intelligence skills in the skill registry.
async fn register_code_tools(state: &AppState, config: &Config) {
    let tools = CodeTools::new(config.code_tools.clone());
    let mut skills = state.skills.write().await;

    let code_skills = [
        (
            "code.scan_repo",
            "Scan a directory tree and return file metadata with detected languages",
        ),
        (
            "code.read_file",
            "Read a file and return its content with language detection",
        ),
        (
            "code.parse_file",
            "Parse a source file with tree-sitter and return its code structure",
        ),
    ];

    for (name, desc) in code_skills {
        if let Err(e) = skills.register(
            name,
            desc,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            serde_json::json!({"type": "object"}),
            SkillProvider::BuiltIn,
        ) {
            tracing::warn!(skill = name, error = %e, "failed to register built-in skill");
        }
    }

    // Store the CodeTools instance in AppState for later invocation.
    drop(skills);
    *state.code_tools.write().await = Some(tools);
}

/// Resolves when Ctrl-C is received.
async fn shutdown_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %e, "failed to install Ctrl+C handler");
    } else {
        info!("shutdown signal received");
    }
}
