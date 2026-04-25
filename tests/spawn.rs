//! Smoke test for the [`tests::common::TestDaemon`] fixture.
//!
//! Confirms the spawn helper actually brings the daemon up to a state where
//! the Unix socket and gRPC port are both connectable, and the auth token
//! has been generated. Subsequent integration tests build on this.

mod common;

use common::{TestDaemon, unix_get};

#[tokio::test]
async fn daemon_starts_and_health_responds() {
    let daemon = TestDaemon::spawn().await;

    assert!(
        daemon.socket_path.exists(),
        "socket path must exist after spawn"
    );
    assert!(
        !daemon.auth_token.is_empty(),
        "auth token file must have been generated"
    );
    assert_eq!(
        daemon.auth_token.len(),
        36,
        "UUID v4 token must be 36 chars, got '{}'",
        daemon.auth_token
    );

    let (status, body) = unix_get(&daemon.socket_path, "/health").await;
    assert_eq!(status, 200, "health endpoint must return 200");
    let v: serde_json::Value = serde_json::from_slice(&body).expect("body must be JSON");
    assert_eq!(
        v.get("status").and_then(|s| s.as_str()),
        Some("ok"),
        "body must report status=ok"
    );

    daemon.shutdown().await.expect("daemon must stop cleanly");
}

#[tokio::test]
async fn daemon_starts_with_auth_disabled() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;
    let (status, _) = unix_get(&daemon.socket_path, "/health").await;
    assert_eq!(status, 200);
    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn two_daemons_can_run_in_parallel() {
    // Fixture must not collide on socket paths or gRPC ports.
    let a = TestDaemon::spawn().await;
    let b = TestDaemon::spawn().await;

    assert_ne!(
        a.socket_path, b.socket_path,
        "tempdirs must be unique per fixture"
    );
    assert_ne!(
        a.grpc_port, b.grpc_port,
        "gRPC ports must be unique per fixture"
    );

    let (sa, _) = unix_get(&a.socket_path, "/health").await;
    let (sb, _) = unix_get(&b.socket_path, "/health").await;
    assert_eq!((sa, sb), (200, 200));

    a.shutdown().await.expect("a shutdown");
    b.shutdown().await.expect("b shutdown");
}
