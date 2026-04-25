// Integration-level smoke test for the core domain types.
// Requires `pub mod core;` in src/lib.rs (and `pub mod error;` if not present).
use chrono::Utc;
use trumpet::core::{
    AgentId, AgentInfo, AgentStatus, ChatMessage, Conversation, ConversationId, MessageId,
};

#[test]
fn core_types_are_accessible_from_library_root() {
    let agent_id = AgentId::new();
    let conv_id = ConversationId::new();
    let msg_id = MessageId::new();

    // All IDs must be distinct — each is a fresh UUIDv4 draw.
    assert_ne!(agent_id.to_string(), conv_id.to_string());
    assert_ne!(conv_id.to_string(), msg_id.to_string());
}

#[test]
fn agent_info_constructs_and_serializes() {
    let info = AgentInfo {
        id: AgentId::new(),
        name: "integration-test-agent".to_owned(),
        registered_at: Utc::now(),
        status: AgentStatus::Connected,
        capabilities: None,
    };
    let json = serde_json::to_string(&info).expect("AgentInfo must serialize");
    assert!(
        json.contains("integration-test-agent"),
        "name must appear in JSON: {json}"
    );
    assert!(
        json.contains("connected"),
        "status must serialize as snake_case: {json}"
    );
}

#[test]
fn conversation_constructs_correctly() {
    let id = ConversationId::new();
    let participant = AgentId::new();
    let conv = Conversation {
        id,
        name: Some("test-convo".to_owned()),
        participants: [participant].into_iter().collect(),
        created_at: Utc::now(),
    };
    assert_eq!(conv.participants.len(), 1);
    assert_eq!(conv.name.as_deref(), Some("test-convo"));
}

#[test]
fn chat_message_constructs_correctly() {
    let conv_id = ConversationId::new();
    let sender = AgentId::new();
    let msg = ChatMessage {
        id: MessageId::new(),
        conversation_id: conv_id,
        sender,
        content: "hello from integration test".to_owned(),
        timestamp: Utc::now(),
    };
    assert_eq!(msg.content, "hello from integration test");
    assert_eq!(msg.conversation_id, conv_id);
    assert_eq!(msg.sender, sender);
}
