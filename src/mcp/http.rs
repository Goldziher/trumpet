//! HTTP transport adapter for the Trumpet MCP server.
//!
//! Wraps [`TrumpetMcpServer`] in rmcp's streamable-HTTP transport so MCP
//! clients can connect via HTTP (over the daemon's Unix socket) rather than
//! stdio. The returned tower service is mounted under `/mcp` by the daemon's
//! axum router.

use std::sync::Arc;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use tokio_util::sync::CancellationToken;

use super::handler::TrumpetMcpServer;

/// Build a [`StreamableHttpService`] backed by [`TrumpetMcpServer`].
///
/// The session manager is local (in-memory). The streamable-HTTP service
/// shares the supplied cancellation token, so daemon shutdown drains active
/// MCP sessions instead of leaving them hanging.
///
/// `Host`-header validation is loosened to accept any `localhost` /
/// `127.0.0.1` / `::1` host, which is appropriate for a service exposed over
/// a Unix socket where the `Host` header is set by the client and is not
/// meaningful for security.
pub fn build_service(
    server: TrumpetMcpServer,
    cancel: CancellationToken,
) -> StreamableHttpService<TrumpetMcpServer, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default().with_cancellation_token(cancel);

    StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}
