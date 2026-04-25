//! End-to-end HTTP integration tests for the Trumpet daemon.
//!
//! These tests exercise the axum router in-process via `tower::ServiceExt`,
//! which is faster and more deterministic than spinning up a real Unix-socket
//! listener. Auth is intentionally skipped here (the listener wrapper that
//! enforces it lives one layer below the router); the unit tests in
//! `src/server/auth.rs` cover the auth surface separately.

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::json;
use tower::ServiceExt as _;

use trumpet::config::Config;
use trumpet::server::AppState;

/// Build an axum Router wired against a fresh in-memory `AppState`.
///
/// The router itself lives in a private module; we go through the public
/// [`trumpet::server::router_for_tests`] shim. Auth is intentionally not
/// applied here (the listener wrapper that enforces it lives one layer
/// below the router); the unit tests in `src/server/auth.rs` cover the
/// auth surface separately.
fn router() -> axum::Router {
    trumpet::server::router_for_tests(AppState::new(Config::default()))
}

async fn read_body(body: Body) -> Vec<u8> {
    to_bytes(body, 1024 * 1024).await.unwrap().to_vec()
}

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let app = router();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body(resp.into_body()).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("ok"));
}

#[tokio::test]
async fn register_then_list_agents() {
    let app = router();

    // Register.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents/register")
                .header("content-type", "application/json")
                .body(Body::from(json!({"name": "worker"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // List.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/agents")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body(resp.into_body()).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let agents = v.as_array().expect("response must be an array");
    assert_eq!(agents.len(), 1, "must contain the just-registered agent");
    assert_eq!(
        agents[0].get("name").and_then(|n| n.as_str()),
        Some("worker")
    );
}

#[tokio::test]
async fn submit_then_list_tasks() {
    let app = router();

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tasks")
                .header("content-type", "application/json")
                .body(Body::from(json!({"message": "do a thing"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/tasks")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = read_body(resp.into_body()).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v.as_array().map(|a| a.len()), Some(1));
}

#[tokio::test]
async fn unknown_tool_returns_404() {
    let app = router();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tools/nonexistent.tool/invoke")
                .header("content-type", "application/json")
                .body(Body::from(json!({"input": {}}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn missing_required_body_field_returns_422() {
    let app = router();
    // /agents/register requires `name`; sending an empty object should fail
    // serde decoding and return 422 from axum's Json extractor.
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/agents/register")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
