//! End-to-end tests for the daemon's gRPC (A2A) surface (F10).
//!
//! Confirms that the bearer-token interceptor enforces auth, that the
//! happy-path SendMessage/GetTask round-trip works, and that error paths
//! return the expected `tonic::Code`.

mod common;

use std::time::Duration;

use common::TestDaemon;
use tonic::Request;
use tonic::transport::Endpoint;

use trumpet::grpc::proto::a2a_service_client::A2aServiceClient;
use trumpet::grpc::proto::{
    GetTaskRequest, ListTasksRequest, Message, Part as ProtoPart, Role, SendMessageRequest,
    part as proto_part,
};

async fn connect(daemon: &TestDaemon) -> tonic::transport::Channel {
    let endpoint = Endpoint::from_shared(format!("http://{}", daemon.grpc_addr))
        .expect("endpoint")
        .connect_timeout(Duration::from_secs(2));
    endpoint.connect().await.expect("connect grpc")
}

fn token_interceptor(
    token: String,
) -> impl FnMut(Request<()>) -> Result<Request<()>, tonic::Status> + Clone {
    move |mut req: Request<()>| {
        req.metadata_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        Ok(req)
    }
}

#[tokio::test]
async fn list_tasks_without_token_returns_unauthenticated() {
    let daemon = TestDaemon::builder().require_auth(true).spawn().await;
    let channel = connect(&daemon).await;

    let mut client = A2aServiceClient::new(channel);
    let result = client
        .list_tasks(Request::new(ListTasksRequest::default()))
        .await;

    assert!(
        matches!(result, Err(ref s) if s.code() == tonic::Code::Unauthenticated),
        "missing bearer token must produce Unauthenticated, got: {result:?}"
    );

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn send_message_then_get_task_round_trips() {
    let daemon = TestDaemon::builder().require_auth(true).spawn().await;
    let channel = connect(&daemon).await;
    let mut client =
        A2aServiceClient::with_interceptor(channel, token_interceptor(daemon.auth_token.clone()));

    let msg = Message {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: Role::User as i32,
        parts: vec![ProtoPart {
            content: Some(proto_part::Content::Text("hello via grpc".into())),
            ..Default::default()
        }],
        ..Default::default()
    };

    let resp = client
        .send_message(Request::new(SendMessageRequest {
            tenant: String::new(),
            message: Some(msg),
            configuration: None,
            metadata: None,
        }))
        .await
        .expect("send_message must succeed");

    let task_id = resp
        .into_inner()
        .payload
        .and_then(|p| match p {
            trumpet::grpc::proto::send_message_response::Payload::Task(t) => Some(t.id),
            _ => None,
        })
        .expect("send_message must return a Task");

    let task = client
        .get_task(Request::new(GetTaskRequest {
            tenant: String::new(),
            id: task_id.clone(),
            history_length: None,
        }))
        .await
        .expect("get_task must succeed")
        .into_inner();
    assert_eq!(task.id, task_id, "GetTask must echo the same task id");

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn get_task_with_unknown_id_returns_not_found() {
    let daemon = TestDaemon::builder().require_auth(true).spawn().await;
    let channel = connect(&daemon).await;
    let mut client =
        A2aServiceClient::with_interceptor(channel, token_interceptor(daemon.auth_token.clone()));

    let result = client
        .get_task(Request::new(GetTaskRequest {
            tenant: String::new(),
            id: uuid::Uuid::new_v4().to_string(),
            history_length: None,
        }))
        .await;

    assert!(
        matches!(result, Err(ref s) if s.code() == tonic::Code::NotFound),
        "GetTask with unknown id must return NotFound, got: {result:?}"
    );

    daemon.shutdown().await.expect("clean shutdown");
}
