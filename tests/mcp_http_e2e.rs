//! End-to-end tests for the MCP streamable-HTTP transport (F6).
//!
//! Exercises the rmcp HTTP transport over the daemon's Unix socket. We talk
//! to it with raw HTTP requests so the test suite stays free of an MCP
//! client crate dependency. The streamable-HTTP server returns SSE-framed
//! JSON-RPC responses; [`common::extract_sse_data`] strips the framing.

mod common;

use serde_json::Value;

use common::{TestDaemon, extract_sse_data, unix_request_with_headers};
use trumpet::config::types::McpTransport;

const ACCEPT_HEADER: &str = "application/json, text/event-stream";

fn initialize_request() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "trumpet-e2e", "version": "0"},
        },
    }))
    .unwrap()
}

fn list_tools_request(id: u64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
    }))
    .unwrap()
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[tokio::test]
async fn mcp_http_initialize_returns_session_id() {
    let daemon = TestDaemon::builder()
        .mcp_enabled(true)
        .mcp_transport(McpTransport::Http)
        .spawn()
        .await;

    let (status, headers, body) = unix_request_with_headers(
        &daemon.socket_path,
        "POST",
        "/mcp",
        &[
            ("Accept", ACCEPT_HEADER),
            ("Content-Type", "application/json"),
        ],
        &initialize_request(),
    )
    .await;

    assert_eq!(status, 200, "initialize must succeed: {body:?}");
    let session =
        header_value(&headers, "mcp-session-id").expect("server must return Mcp-Session-Id header");
    assert!(!session.is_empty(), "session id must not be empty");

    let json_str = extract_sse_data(&body);
    let resp: Value = serde_json::from_str(&json_str).expect("init response is JSON");
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(
        resp["result"]["serverInfo"]["name"], "trumpet",
        "init response must echo the server name; got: {resp}"
    );

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn mcp_http_tools_list_returns_built_in_tools() {
    let daemon = TestDaemon::builder()
        .mcp_enabled(true)
        .mcp_transport(McpTransport::Http)
        .spawn()
        .await;

    // 1) initialize, capture session id
    let (init_status, init_headers, _) = unix_request_with_headers(
        &daemon.socket_path,
        "POST",
        "/mcp",
        &[
            ("Accept", ACCEPT_HEADER),
            ("Content-Type", "application/json"),
        ],
        &initialize_request(),
    )
    .await;
    assert_eq!(init_status, 200, "initialize must succeed");
    let session = header_value(&init_headers, "mcp-session-id")
        .expect("Mcp-Session-Id header")
        .to_owned();

    // 2) Send the `notifications/initialized` notification (required by spec).
    let initialized = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    }))
    .unwrap();
    let (notif_status, _, _) = unix_request_with_headers(
        &daemon.socket_path,
        "POST",
        "/mcp",
        &[
            ("Accept", ACCEPT_HEADER),
            ("Content-Type", "application/json"),
            ("Mcp-Session-Id", &session),
        ],
        &initialized,
    )
    .await;
    assert_eq!(
        notif_status, 202,
        "initialized notification must be accepted (202)"
    );

    // 3) tools/list with the session id.
    let (status, _, body) = unix_request_with_headers(
        &daemon.socket_path,
        "POST",
        "/mcp",
        &[
            ("Accept", ACCEPT_HEADER),
            ("Content-Type", "application/json"),
            ("Mcp-Session-Id", &session),
        ],
        &list_tools_request(2),
    )
    .await;
    assert_eq!(status, 200, "tools/list must succeed: {body:?}");

    let json_str = extract_sse_data(&body);
    let resp: Value = serde_json::from_str(&json_str).expect("tools/list response is JSON");
    let tools = resp["result"]["tools"]
        .as_array()
        .expect("tools array in response");

    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        names.contains(&"register_agent"),
        "tools/list must contain register_agent: got {names:?}"
    );
    assert!(
        names.contains(&"list_agents"),
        "tools/list must contain list_agents: got {names:?}"
    );
    assert!(
        names.contains(&"submit_task"),
        "tools/list must contain submit_task: got {names:?}"
    );
    // Built-in code tools must surface dynamically (proves dynamic-tool wiring).
    assert!(
        names.contains(&"code.scan_repo"),
        "tools/list must contain dynamic code.scan_repo: got {names:?}"
    );

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn mcp_http_get_without_session_returns_400() {
    let daemon = TestDaemon::builder()
        .mcp_enabled(true)
        .mcp_transport(McpTransport::Http)
        .spawn()
        .await;

    let (status, _, _) = unix_request_with_headers(
        &daemon.socket_path,
        "GET",
        "/mcp",
        &[("Accept", "text/event-stream")],
        &[],
    )
    .await;
    assert_eq!(
        status, 400,
        "GET /mcp without a session id must be rejected with 400"
    );

    daemon.shutdown().await.expect("clean shutdown");
}
