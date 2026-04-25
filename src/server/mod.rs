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

use rmcp::ServiceExt as _;

use crate::config::Config;
use crate::config::types::McpTransport;
use crate::core::code_tools::CodeTools;
use crate::core::types::ToolProvider;
use crate::error::Result;
use crate::grpc::proto::a2a_service_server::A2aServiceServer;
use crate::grpc::service::NexusA2aService;
use crate::state::StateManager;

/// Start the Trumpet daemon.
///
/// Binds a Unix domain socket for HTTP, starts gRPC on the configured TCP
/// port, restores persisted state, registers built-in tools, spawns a
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

    // ── Register built-in code tools ───────────────────────────────────────
    register_code_tools(&state, config).await?;

    // ── gRPC server ──────────────────────────────────────────────────────────
    let grpc_addr = format!("{}:{}", config.server.host, config.server.grpc_port);
    let grpc_socket_addr: std::net::SocketAddr =
        grpc_addr
            .parse()
            .map_err(|e| crate::error::Error::DaemonBindFailed {
                path: grpc_addr.clone(),
                reason: format!("invalid gRPC listen address: {e}"),
            })?;
    let grpc_service = NexusA2aService::new(state.clone());
    let grpc_handle = tokio::spawn(async move {
        info!(addr = %grpc_socket_addr, "gRPC server starting");
        if let Err(e) = tonic::transport::Server::builder()
            .add_service(A2aServiceServer::new(grpc_service))
            .serve(grpc_socket_addr)
            .await
        {
            tracing::error!(error = %e, "gRPC server failed");
        }
    });

    // ── MCP server ───────────────────────────────────────────────────────────
    let mcp_handle = if config.mcp.enabled {
        match config.mcp.transport {
            McpTransport::Stdio => {
                let mcp_server = crate::mcp::handler::TrumpetMcpServer::new(state.clone());
                Some(tokio::spawn(async move {
                    info!("MCP server starting on stdio");
                    let transport = rmcp::transport::io::stdio();
                    match mcp_server.serve(transport).await {
                        Ok(running) => {
                            if let Err(e) = running.waiting().await {
                                tracing::error!(error = %e, "MCP stdio server stopped with error");
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "MCP stdio server failed to start");
                        }
                    }
                }))
            }
            McpTransport::Http => {
                tracing::warn!("MCP HTTP transport not yet implemented; skipping MCP server");
                None
            }
        }
    } else {
        tracing::info!("MCP server disabled by configuration");
        None
    };

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
    if let Some(handle) = mcp_handle {
        handle.abort();
    }

    info!("saving state snapshot before shutdown");
    let snapshot = state.to_snapshot().await;
    if let Err(e) = state_manager.save_snapshot(&snapshot).await {
        tracing::error!(error = %e, "failed to save shutdown snapshot");
    }

    let _ = tokio::fs::remove_file(socket_path).await;
    info!("trumpet daemon stopped");

    Ok(())
}

/// Register built-in code intelligence tools in the tool registry.
async fn register_code_tools(state: &AppState, config: &Config) -> Result<()> {
    let code_tools = CodeTools::new(config.code_tools.clone())?;
    let mut tools = state.tools.write().await;

    let builtin_tools = [
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

    for (name, desc) in builtin_tools {
        if let Err(e) = tools.register(
            name,
            desc,
            serde_json::json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            serde_json::json!({"type": "object"}),
            ToolProvider::BuiltIn,
        ) {
            tracing::warn!(tool = name, error = %e, "failed to register built-in tool");
        }
    }

    // Store the CodeTools instance in AppState for later invocation.
    drop(tools);
    *state.code_tools.write().await = Some(code_tools);
    Ok(())
}

/// Resolves when Ctrl-C or SIGTERM is received.
///
/// On Unix, races Ctrl-C against SIGTERM via `tokio::select!`. If SIGTERM
/// registration fails (rare, but possible under restrictive seccomp profiles
/// or low file-descriptor limits), the function logs a warning and falls back
/// to Ctrl-C-only behaviour rather than panicking and crashing the daemon.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let ctrl_c = tokio::signal::ctrl_c();
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => {
                tokio::select! {
                    result = ctrl_c => {
                        if let Err(e) = result {
                            tracing::error!(error = %e, "Ctrl+C handler failed");
                            return;
                        }
                    }
                    _ = sigterm.recv() => {}
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "failed to install SIGTERM handler; falling back to Ctrl+C only"
                );
                if let Err(err) = ctrl_c.await {
                    tracing::error!(error = %err, "Ctrl+C handler failed");
                    return;
                }
            }
        }
    }

    #[cfg(not(unix))]
    {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "Ctrl+C handler failed");
            return;
        }
    }

    info!("shutdown signal received");
}
