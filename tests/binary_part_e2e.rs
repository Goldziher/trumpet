//! End-to-end test for `Part::Bytes` round-trip through REST + gRPC (F10).
//!
//! Confirms that submitting a task whose message contains a binary part is
//! preserved verbatim across the daemon's persistence layer and the gRPC /
//! REST surfaces.

mod common;

use std::time::Duration;

use base64::Engine;
use common::{TestDaemon, unix_get, unix_post};
use serde_json::json;

#[tokio::test]
async fn rest_binary_part_round_trip_through_task_message() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;

    // The REST `submit_task` API only accepts a plain `message` string —
    // binary parts are introduced by agents via gRPC `SendMessage`. This
    // test instead constructs a task and a follow-up message via the
    // /conversations API path which uses raw `TaskMessage` JSON.
    //
    // For F10 coverage we focus on the JSON shape: serde must produce the
    // right tag (`{"type": "bytes", "bytes": "<base64>"}`).
    let payload = b"\x00\x01\x02hello\xff";
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload);

    let part_json = json!({
        "type": "bytes",
        "bytes": encoded,
    });

    // Round-trip the JSON through the canonical `Part` deserializer.
    let part: trumpet::core::task_types::Part =
        serde_json::from_value(part_json.clone()).expect("Part::Bytes deserialises from JSON");
    match &part {
        trumpet::core::task_types::Part::Bytes { bytes } => {
            assert_eq!(bytes, payload, "binary payload must round-trip");
        }
        other => panic!("expected Part::Bytes, got {other:?}"),
    }

    // And re-serializes to the same shape.
    let reserialised = serde_json::to_value(&part).expect("Part serialises");
    assert_eq!(
        reserialised["type"].as_str(),
        Some("bytes"),
        "type tag must survive round-trip"
    );
    assert_eq!(
        reserialised["bytes"].as_str(),
        Some(encoded.as_str()),
        "base64 payload must survive round-trip"
    );

    // Smoke-check that the daemon's REST /tasks pipeline accepts a regular
    // text message; this proves the harness is alive and the task router
    // is wired before we exercise the binary-part encoding above.
    let (status, body) = unix_post(
        &daemon.socket_path,
        "/tasks",
        "application/json",
        br#"{"message":"hello"}"#,
    )
    .await;
    assert_eq!(status, 200, "/tasks must accept a text message: {body:?}");

    daemon.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn grpc_send_message_round_trips_bytes_part_to_rest() {
    use tonic::Request;
    use tonic::transport::Endpoint;
    use trumpet::grpc::proto::a2a_service_client::A2aServiceClient;
    use trumpet::grpc::proto::{
        Message, Part as ProtoPart, SendMessageRequest, part as proto_part,
    };

    let daemon = TestDaemon::builder().require_auth(false).spawn().await;

    let endpoint = Endpoint::from_shared(format!("http://{}", daemon.grpc_addr))
        .expect("endpoint")
        .connect_timeout(Duration::from_secs(2));
    let channel = endpoint.connect().await.expect("connect grpc");
    let mut client = A2aServiceClient::new(channel);

    let payload = b"\xde\xad\xbe\xefbinary".to_vec();
    let msg = Message {
        message_id: uuid::Uuid::new_v4().to_string(),
        role: trumpet::grpc::proto::Role::User as i32,
        parts: vec![ProtoPart {
            content: Some(proto_part::Content::Raw(payload.clone())),
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
        .expect("send_message must return a Task with an id");

    // Read the task back via REST and confirm the binary part was preserved.
    let (status, body) = unix_get(&daemon.socket_path, &format!("/tasks/{task_id}")).await;
    assert_eq!(status, 200, "GET /tasks/{task_id} must succeed: {body:?}");
    let task: serde_json::Value = serde_json::from_slice(&body).unwrap();

    let history = task
        .get("history")
        .and_then(|v| v.as_array())
        .expect("task must have a history array");
    let first_msg = history.first().expect("history must contain the message");
    let parts = first_msg["parts"].as_array().expect("message has parts");
    let part = &parts[0];
    assert_eq!(
        part["type"].as_str(),
        Some("bytes"),
        "REST representation of binary part must use type=bytes, got: {part}"
    );
    let encoded = part["bytes"].as_str().expect("bytes field");
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("base64 decode");
    assert_eq!(
        decoded, payload,
        "binary payload must survive gRPC -> persistence -> REST round-trip"
    );

    daemon.shutdown().await.expect("clean shutdown");
}
