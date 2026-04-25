//! Proto ↔ core type conversion for the A2A gRPC service.
//!
//! All conversions between `proto::*` (prost-generated) and `core::*`
//! (domain) types live here, keeping the service implementation clean.

use tonic::Status;

use crate::core::task_types::{
    Artifact, MessageRole, Part, Task, TaskMessage, TaskState, TaskStatus,
};
use crate::core::types::MessageId;
use crate::grpc::proto;

// ── TaskState ───────────────────────────────────────────────────────────────

/// Convert a core [`TaskState`] to the proto enum value.
pub fn core_state_to_proto(state: TaskState) -> i32 {
    match state {
        TaskState::Submitted => proto::TaskState::Submitted.into(),
        TaskState::Working => proto::TaskState::Working.into(),
        TaskState::Completed => proto::TaskState::Completed.into(),
        TaskState::Failed => proto::TaskState::Failed.into(),
        TaskState::Canceled => proto::TaskState::Canceled.into(),
        TaskState::InputRequired => proto::TaskState::InputRequired.into(),
        TaskState::Rejected => proto::TaskState::Rejected.into(),
        TaskState::AuthRequired => proto::TaskState::AuthRequired.into(),
    }
}

/// Convert a proto task state i32 to a core [`TaskState`].
pub fn proto_state_to_core(value: i32) -> Result<TaskState, Status> {
    match proto::TaskState::try_from(value) {
        Ok(proto::TaskState::Submitted) => Ok(TaskState::Submitted),
        Ok(proto::TaskState::Working) => Ok(TaskState::Working),
        Ok(proto::TaskState::Completed) => Ok(TaskState::Completed),
        Ok(proto::TaskState::Failed) => Ok(TaskState::Failed),
        Ok(proto::TaskState::Canceled) => Ok(TaskState::Canceled),
        Ok(proto::TaskState::InputRequired) => Ok(TaskState::InputRequired),
        Ok(proto::TaskState::Rejected) => Ok(TaskState::Rejected),
        Ok(proto::TaskState::AuthRequired) => Ok(TaskState::AuthRequired),
        Ok(proto::TaskState::Unspecified) | Err(_) => Err(Status::invalid_argument(format!(
            "unknown task state: {value}"
        ))),
    }
}

// ── Role ────────────────────────────────────────────────────────────────────

fn core_role_to_proto(role: &MessageRole) -> i32 {
    match *role {
        MessageRole::User => proto::Role::User.into(),
        MessageRole::Agent => proto::Role::Agent.into(),
    }
}

fn proto_role_to_core(value: i32) -> Result<MessageRole, Status> {
    match proto::Role::try_from(value) {
        Ok(proto::Role::User) => Ok(MessageRole::User),
        Ok(proto::Role::Agent) => Ok(MessageRole::Agent),
        _ => Err(Status::invalid_argument(format!("unknown role: {value}"))),
    }
}

// ── Part ────────────────────────────────────────────────────────────────────

fn core_part_to_proto(part: &Part) -> proto::Part {
    let content = match part {
        Part::Text { text } => Some(proto::part::Content::Text(text.clone())),
        Part::Url { url } => Some(proto::part::Content::Url(url.clone())),
        Part::Data { data } => {
            let prost_val = json_to_prost_value(data.clone());
            Some(proto::part::Content::Data(prost_val))
        }
    };
    proto::Part {
        content,
        metadata: None,
        filename: String::new(),
        media_type: String::new(),
    }
}

fn proto_part_to_core(part: &proto::Part) -> Result<Part, Status> {
    match &part.content {
        Some(proto::part::Content::Text(text)) => Ok(Part::Text { text: text.clone() }),
        Some(proto::part::Content::Url(url)) => Ok(Part::Url { url: url.clone() }),
        Some(proto::part::Content::Data(val)) => {
            let json = prost_value_to_json(val);
            Ok(Part::Data { data: json })
        }
        Some(proto::part::Content::Raw(bytes)) => Err(Status::unimplemented(format!(
            "binary (raw) parts are not yet supported ({} bytes)",
            bytes.len()
        ))),
        None => Err(Status::invalid_argument("part has no content")),
    }
}

// ── Message ─────────────────────────────────────────────────────────────────

/// Convert a core [`TaskMessage`] to a proto [`Message`].
pub fn core_message_to_proto(msg: &TaskMessage) -> proto::Message {
    proto::Message {
        message_id: msg.id.to_string(),
        context_id: String::new(),
        task_id: String::new(),
        role: core_role_to_proto(&msg.role),
        parts: msg.parts.iter().map(core_part_to_proto).collect(),
        metadata: msg
            .metadata
            .as_ref()
            .map(|m| json_to_prost_struct(m.clone())),
        extensions: vec![],
        reference_task_ids: vec![],
    }
}

/// Convert a proto [`Message`] to a core [`TaskMessage`].
pub fn proto_message_to_core(msg: &proto::Message) -> Result<TaskMessage, Status> {
    let role = proto_role_to_core(msg.role)?;
    let parts: Result<Vec<Part>, Status> = msg.parts.iter().map(proto_part_to_core).collect();
    let metadata = msg.metadata.as_ref().map(prost_struct_to_json);

    Ok(TaskMessage {
        id: if msg.message_id.is_empty() {
            MessageId::new()
        } else {
            msg.message_id
                .parse()
                .map_err(|_| Status::invalid_argument("invalid message_id UUID"))?
        },
        role,
        parts: parts?,
        metadata,
    })
}

// ── TaskStatus ──────────────────────────────────────────────────────────────

fn core_status_to_proto(status: &TaskStatus) -> proto::TaskStatus {
    proto::TaskStatus {
        state: core_state_to_proto(status.state),
        message: status.message.as_ref().map(core_message_to_proto),
        timestamp: Some(datetime_to_timestamp(status.timestamp)),
    }
}

// ── Artifact ────────────────────────────────────────────────────────────────

fn core_artifact_to_proto(artifact: &Artifact) -> proto::Artifact {
    proto::Artifact {
        artifact_id: artifact.id.to_string(),
        name: artifact.name.clone().unwrap_or_default(),
        description: artifact.description.clone().unwrap_or_default(),
        parts: artifact.parts.iter().map(core_part_to_proto).collect(),
        metadata: artifact
            .metadata
            .as_ref()
            .map(|m| json_to_prost_struct(m.clone())),
        extensions: vec![],
    }
}

// ── Task ────────────────────────────────────────────────────────────────────

/// Convert a core [`Task`] to a proto [`Task`].
pub fn core_task_to_proto(task: &Task) -> proto::Task {
    proto::Task {
        id: task.id.to_string(),
        context_id: task.context_id.to_string(),
        status: Some(core_status_to_proto(&task.status)),
        artifacts: task.artifacts.iter().map(core_artifact_to_proto).collect(),
        history: task.history.iter().map(core_message_to_proto).collect(),
        metadata: task
            .metadata
            .as_ref()
            .map(|m| json_to_prost_struct(m.clone())),
    }
}

// ── Timestamp helpers ───────────────────────────────────────────────────────

fn datetime_to_timestamp(dt: chrono::DateTime<chrono::Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

// ── JSON ↔ prost_types helpers ──────────────────────────────────────────────

fn json_to_prost_struct(val: serde_json::Value) -> prost_types::Struct {
    match val {
        serde_json::Value::Object(map) => prost_types::Struct {
            fields: map
                .into_iter()
                .map(|(k, v)| (k, json_to_prost_value(v)))
                .collect(),
        },
        _ => prost_types::Struct::default(),
    }
}

fn json_to_prost_value(val: serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match val {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s),
        serde_json::Value::Array(arr) => Kind::ListValue(prost_types::ListValue {
            values: arr.into_iter().map(json_to_prost_value).collect(),
        }),
        serde_json::Value::Object(map) => Kind::StructValue(prost_types::Struct {
            fields: map
                .into_iter()
                .map(|(k, v)| (k, json_to_prost_value(v)))
                .collect(),
        }),
    };
    prost_types::Value { kind: Some(kind) }
}

fn prost_struct_to_json(s: &prost_types::Struct) -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = s
        .fields
        .iter()
        .map(|(k, v)| (k.clone(), prost_value_to_json(v)))
        .collect();
    serde_json::Value::Object(map)
}

fn prost_value_to_json(v: &prost_types::Value) -> serde_json::Value {
    use prost_types::value::Kind;
    match &v.kind {
        Some(Kind::NullValue(_)) => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => serde_json::Number::from_f64(*n)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::ListValue(list)) => {
            serde_json::Value::Array(list.values.iter().map(prost_value_to_json).collect())
        }
        Some(Kind::StructValue(s)) => prost_struct_to_json(s),
        None => serde_json::Value::Null,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task_types::{ContextId, TaskId};

    #[test]
    fn state_round_trip() {
        let states = [
            TaskState::Submitted,
            TaskState::Working,
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Canceled,
            TaskState::InputRequired,
            TaskState::Rejected,
            TaskState::AuthRequired,
        ];
        for state in states {
            let proto_val = core_state_to_proto(state);
            let back = proto_state_to_core(proto_val).expect("round-trip must succeed");
            assert_eq!(back, state, "state must survive round-trip");
        }
    }

    #[test]
    fn json_struct_round_trip() {
        let original = serde_json::json!({"key": "value", "num": 42.0, "nested": {"a": true}});
        let prost = json_to_prost_struct(original.clone());
        let back = prost_struct_to_json(&prost);
        assert_eq!(back, original, "JSON struct must survive prost round-trip");
    }

    #[test]
    fn part_text_round_trip() {
        let core_part = Part::Text {
            text: "hello".to_owned(),
        };
        let proto_part = core_part_to_proto(&core_part);
        let back = proto_part_to_core(&proto_part).expect("round-trip must succeed");
        assert_eq!(back, core_part, "text part must survive round-trip");
    }

    #[test]
    fn message_converts_to_proto() {
        let msg = TaskMessage {
            id: MessageId::new(),
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: "test".to_owned(),
            }],
            metadata: None,
        };
        let proto_msg = core_message_to_proto(&msg);
        assert_eq!(proto_msg.message_id, msg.id.to_string());
        assert_eq!(proto_msg.role, proto::Role::User as i32);
        assert_eq!(proto_msg.parts.len(), 1);
    }

    #[test]
    fn task_converts_to_proto() {
        let task = Task {
            id: TaskId::new(),
            context_id: ContextId::new(),
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: chrono::Utc::now(),
            },
            artifacts: vec![],
            history: vec![],
            metadata: None,
            assignee: None,
            creator: None,
        };
        let proto_task = core_task_to_proto(&task);
        assert_eq!(proto_task.id, task.id.to_string());
        assert!(proto_task.status.is_some());
    }
}
