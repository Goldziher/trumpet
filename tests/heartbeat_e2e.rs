//! End-to-end tests for the agent heartbeat surface and watchdog (F4).

mod common;

use std::time::Duration;

use serde_json::Value;

use common::{TestDaemon, unix_get, unix_post};

/// Poll until `predicate(value)` returns true, or until the deadline elapses.
async fn wait_for<F: Fn(&Value) -> bool>(
    socket: &std::path::Path,
    path: &str,
    predicate: F,
    timeout: Duration,
) -> Value {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let (status, body) = unix_get(socket, path).await;
        assert_eq!(status, 200, "GET {path} returned {status}");
        let v: Value = serde_json::from_slice(&body).expect("response is JSON");
        if predicate(&v) {
            return v;
        }
        if std::time::Instant::now() > deadline {
            panic!("predicate never matched within {timeout:?}: {v}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn watchdog_flips_unhealthy_agent_to_disconnected() {
    // 1s heartbeat interval (so the watchdog ticks fast), 250ms agent
    // timeout so a freshly registered agent goes stale within the test
    // window.
    let daemon = TestDaemon::builder()
        .heartbeat_interval_secs(1)
        .agent_timeout_secs(1)
        .spawn()
        .await;

    // Register an agent.
    let (status, body) = unix_post(
        &daemon.socket_path,
        "/agents/register",
        "application/json",
        br#"{"name":"unhealthy"}"#,
    )
    .await;
    assert_eq!(status, 200);
    let info: Value = serde_json::from_slice(&body).unwrap();
    let agent_id = info["id"].as_str().expect("id field").to_owned();

    // Confirm initially Connected.
    assert_eq!(
        info.get("status").and_then(|v| v.as_str()),
        Some("connected")
    );

    // Wait for the watchdog to flip it. Agent timeout is 1 s but the
    // watchdog ticks every 1 s, so 3 s is a comfortable upper bound.
    let updated = wait_for(
        &daemon.socket_path,
        "/agents",
        |v| {
            v.as_array()
                .and_then(|arr| arr.iter().find(|a| a["id"] == agent_id))
                .and_then(|a| a["status"].as_str())
                .is_some_and(|s| s == "disconnected")
        },
        Duration::from_secs(5),
    )
    .await;
    let updated_agent = updated
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == agent_id)
        .unwrap();
    assert_eq!(updated_agent["status"], "disconnected");

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn heartbeat_endpoint_keeps_agent_connected_and_can_reconnect() {
    let daemon = TestDaemon::builder()
        .heartbeat_interval_secs(1)
        .agent_timeout_secs(1)
        .spawn()
        .await;

    // Register an agent.
    let (status, body) = unix_post(
        &daemon.socket_path,
        "/agents/register",
        "application/json",
        br#"{"name":"reconnector"}"#,
    )
    .await;
    assert_eq!(status, 200);
    let info: Value = serde_json::from_slice(&body).unwrap();
    let agent_id = info["id"].as_str().expect("id").to_owned();

    // Let the agent go stale.
    wait_for(
        &daemon.socket_path,
        "/agents",
        |v| {
            v.as_array()
                .and_then(|arr| arr.iter().find(|a| a["id"] == agent_id))
                .and_then(|a| a["status"].as_str())
                .is_some_and(|s| s == "disconnected")
        },
        Duration::from_secs(5),
    )
    .await;

    // Heartbeat the agent — must flip it back to Connected.
    let (status, body) = unix_post(
        &daemon.socket_path,
        &format!("/agents/{agent_id}/heartbeat"),
        "application/json",
        b"",
    )
    .await;
    assert_eq!(status, 200, "heartbeat must succeed");
    let after: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(after["status"], "connected", "heartbeat must reconnect");

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn task_with_short_deadline_is_failed_by_watchdog() {
    let daemon = TestDaemon::builder()
        .heartbeat_interval_secs(1)
        .agent_timeout_secs(60)
        .spawn()
        .await;

    // Submit a task with a 200ms deadline.
    let body = serde_json::to_vec(&serde_json::json!({
        "message": "do work",
        "deadline_ms": 200u64,
    }))
    .unwrap();
    let (status, body) = unix_post(&daemon.socket_path, "/tasks", "application/json", &body).await;
    assert_eq!(status, 200);
    let task: Value = serde_json::from_slice(&body).unwrap();
    let task_id = task["id"].as_str().expect("task id").to_owned();

    // Wait for the watchdog to mark it Rejected (Submitted -> Rejected
    // because we have no connected agent to pick it up).
    wait_for(
        &daemon.socket_path,
        &format!("/tasks/{task_id}"),
        |v| {
            v.get("status")
                .and_then(|s| s.get("state"))
                .and_then(|s| s.as_str())
                .is_some_and(|s| s == "rejected" || s == "failed")
        },
        Duration::from_secs(5),
    )
    .await;

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn submit_task_to_disconnected_agent_is_rejected() {
    let daemon = TestDaemon::builder()
        .heartbeat_interval_secs(1)
        .agent_timeout_secs(1)
        .spawn()
        .await;

    let (_, body) = unix_post(
        &daemon.socket_path,
        "/agents/register",
        "application/json",
        br#"{"name":"offline"}"#,
    )
    .await;
    let info: Value = serde_json::from_slice(&body).unwrap();
    let agent_id = info["id"].as_str().unwrap().to_owned();

    // Wait for disconnect.
    wait_for(
        &daemon.socket_path,
        "/agents",
        |v| {
            v.as_array()
                .and_then(|arr| arr.iter().find(|a| a["id"] == agent_id))
                .and_then(|a| a["status"].as_str())
                .is_some_and(|s| s == "disconnected")
        },
        Duration::from_secs(5),
    )
    .await;

    // Submitting a task pinned to a disconnected agent must be rejected.
    let body = serde_json::to_vec(&serde_json::json!({
        "message": "do work",
        "assignee": agent_id,
    }))
    .unwrap();
    let (status, resp_body) =
        unix_post(&daemon.socket_path, "/tasks", "application/json", &body).await;
    // SERVICE_UNAVAILABLE = 503 (per Error::AgentDisconnected http_status mapping).
    assert_eq!(
        status,
        503,
        "pinned-disconnected-assignee submission must be rejected: body={}",
        String::from_utf8_lossy(&resp_body)
    );

    daemon.shutdown().await.expect("clean shutdown");
}
