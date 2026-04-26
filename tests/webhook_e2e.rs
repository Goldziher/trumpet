//! End-to-end test for push-notification webhook delivery (F10).
//!
//! Spawns a `wiremock` HTTP mock, registers a webhook against a task via
//! the gRPC `CreateTaskPushNotificationConfig` RPC, then drives the task
//! through a state change and asserts that wiremock received the expected
//! POST.

mod common;

use std::time::Duration;

use common::{TestDaemon, unix_post};
use serde_json::Value;
use tonic::Request;
use tonic::transport::Endpoint;
use trumpet::grpc::proto::AuthenticationInfo;
use trumpet::grpc::proto::TaskPushNotificationConfig;
use trumpet::grpc::proto::a2a_service_client::A2aServiceClient;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn task_status_change_delivers_webhook() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;

    // Stand up wiremock; match any POST and respond 200.
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock_server)
        .await;
    let webhook_url = format!("{}/hook", mock_server.uri());

    // Submit a task via REST so we have a stable task id.
    let (status, body) = unix_post(
        &daemon.socket_path,
        "/tasks",
        "application/json",
        br#"{"message":"webhook-target"}"#,
    )
    .await;
    assert_eq!(status, 200, "/tasks submit must succeed");
    let task: Value = serde_json::from_slice(&body).unwrap();
    let task_id = task["id"].as_str().expect("task id").to_owned();

    // Register the webhook via gRPC.
    let endpoint = Endpoint::from_shared(format!("http://{}", daemon.grpc_addr))
        .expect("endpoint")
        .connect_timeout(Duration::from_secs(2));
    let channel = endpoint.connect().await.expect("connect grpc");
    let mut grpc = A2aServiceClient::new(channel);

    grpc.create_task_push_notification_config(Request::new(TaskPushNotificationConfig {
        tenant: String::new(),
        task_id: task_id.clone(),
        url: webhook_url.clone(),
        token: "shared-secret".into(),
        authentication: Some(AuthenticationInfo {
            scheme: "bearer".into(),
            credentials: "fake".into(),
        }),
        id: String::new(),
    }))
    .await
    .expect("register webhook must succeed");

    // Cancel the task to trigger a TaskStatusUpdated event the worker forwards.
    let (status, _) = unix_post(
        &daemon.socket_path,
        &format!("/tasks/{task_id}/cancel"),
        "application/json",
        b"{}",
    )
    .await;
    assert_eq!(status, 200, "cancel must succeed");

    // Poll wiremock until at least one request was received (or timeout).
    let received = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let requests = mock_server.received_requests().await.unwrap_or_default();
            if !requests.is_empty() {
                return requests.len();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("wiremock must receive a webhook delivery within 5s");

    assert!(
        received >= 1,
        "wiremock must record at least one webhook POST"
    );

    daemon.shutdown().await.expect("clean shutdown");
}
