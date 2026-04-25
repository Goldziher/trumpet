//! MCP server implementation for the Trumpet agent nexus.
//!
//! Exposes [`TrumpetMcpServer`] which wraps [`AppState`] and registers tools
//! for agent management, skill discovery, and conversation handling.

pub mod handler;
