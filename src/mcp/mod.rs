//! MCP server implementation for the Trumpet agent nexus.
//!
//! Exposes [`TrumpetMcpServer`] which wraps [`AppState`] and registers tools
//! for agent management, tool discovery, and conversation handling.

pub mod handler;
pub mod http;
pub mod notifier;
