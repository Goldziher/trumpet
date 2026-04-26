//! End-to-end test for snapshot save/restore (F10).
//!
//! Confirms that state survives a daemon restart: register an agent and
//! submit a task on one daemon, then spawn a fresh daemon against the same
//! storage path and verify both pieces of state are restored from the
//! snapshot.

mod common;

use common::{TestDaemon, unix_get, unix_post};

#[tokio::test]
async fn snapshot_round_trip_restores_agents_and_tasks() {
    // Use a 1-second snapshot interval so the daemon writes a snapshot at
    // least once before we shut it down, but rely on the shutdown-hook
    // snapshot as the authoritative path.
    let storage_path = tempfile::tempdir().expect("storage tempdir");
    let storage = storage_path.path().to_owned();
    let auth_token_path = tempfile::NamedTempFile::new()
        .expect("token tempfile")
        .into_temp_path()
        .to_path_buf();
    // Remove the empty tempfile so load_or_create_token writes a fresh one.
    let _ = std::fs::remove_file(&auth_token_path);

    // ── First daemon: register an agent and submit a task ────────────────
    let daemon = TestDaemon::builder()
        .require_auth(false)
        .storage_path(storage.clone())
        .auth_token_path(auth_token_path.clone())
        .spawn()
        .await;

    let (status, body) = unix_post(
        &daemon.socket_path,
        "/agents/register",
        "application/json",
        br#"{"name":"persistent-worker"}"#,
    )
    .await;
    assert_eq!(status, 200, "register must succeed");
    let agent: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let agent_id = agent["id"].as_str().expect("agent id").to_owned();

    let (status, body) = unix_post(
        &daemon.socket_path,
        "/tasks",
        "application/json",
        br#"{"message":"persisted-task"}"#,
    )
    .await;
    assert_eq!(status, 200, "submit must succeed");
    let task: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let task_id = task["id"].as_str().expect("task id").to_owned();

    daemon.shutdown().await.expect("clean shutdown");

    // ── Second daemon: same storage, verify state restored ───────────────
    let daemon2 = TestDaemon::builder()
        .require_auth(false)
        .storage_path(storage.clone())
        .auth_token_path(auth_token_path.clone())
        .spawn()
        .await;

    // Agent must be in the registry.
    let (status, body) = unix_get(&daemon2.socket_path, "/agents").await;
    assert_eq!(status, 200);
    let agents: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert!(
        agents.iter().any(|a| a["id"].as_str() == Some(&agent_id)),
        "agent {agent_id} must survive a daemon restart, got: {agents:?}"
    );

    // Task must be in the task manager.
    let (status, body) = unix_get(&daemon2.socket_path, &format!("/tasks/{task_id}")).await;
    assert_eq!(
        status, 200,
        "task {task_id} must be retrievable after restart, got status {status} body {body:?}"
    );
    let restored: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        restored["id"].as_str(),
        Some(task_id.as_str()),
        "restored task id must match"
    );

    daemon2.shutdown().await.expect("second clean shutdown");
}
