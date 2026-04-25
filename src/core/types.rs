use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// Unique identifier for a registered agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(Uuid);

impl AgentId {
    /// Create a new random [`AgentId`].
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for AgentId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

/// Unique identifier for a conversation thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(Uuid);

impl ConversationId {
    /// Create a new random [`ConversationId`].
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ConversationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

/// Unique identifier for a single chat message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(Uuid);

impl MessageId {
    /// Create a new random [`MessageId`].
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for MessageId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

/// Unique identifier for a registered tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolId(Uuid);

impl ToolId {
    /// Create a new random [`ToolId`].
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ToolId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ToolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for ToolId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

/// Lifecycle state of a registered agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// The agent is connected and accepting work.
    Connected,
    /// The agent is not currently reachable.
    Disconnected,
}

/// Metadata for a registered agent.
///
/// Does not include runtime state such as active tasks; those live in the
/// registry and are looked up by [`AgentId`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    /// Stable identity for this agent.
    pub id: AgentId,
    /// Human-readable agent name (e.g. `"claude-code-1"`).
    pub name: String,
    /// Wall-clock time at which the agent first registered.
    pub registered_at: DateTime<Utc>,
    /// Current connectivity status.
    pub status: AgentStatus,
}

/// A single message exchanged within a [`Conversation`].
///
/// Messages are stored separately from [`Conversation`] to keep the
/// conversation listing lean.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Stable identity for this message.
    pub id: MessageId,
    /// The conversation this message belongs to.
    pub conversation_id: ConversationId,
    /// The agent that sent this message.
    pub sender: AgentId,
    /// UTF-8 message body.
    pub content: String,
    /// Wall-clock time at which the message was created.
    pub timestamp: DateTime<Utc>,
}

/// Who provides a tool — an agent or the system itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolProvider {
    /// The tool is provided by a connected agent.
    Agent { agent_id: AgentId },
    /// The tool is provided by the daemon itself.
    BuiltIn,
}

/// Metadata for a registered tool.
///
/// `Eq` and `Hash` cannot be derived because `serde_json::Value` contains
/// floats which only implement `PartialEq`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Stable identity for this tool.
    pub id: ToolId,
    /// Human-readable tool name (e.g. `"code.scan_repo"`).
    pub name: String,
    /// Description of what the tool does.
    pub description: String,
    /// JSON Schema describing the expected input payload.
    pub input_schema: serde_json::Value,
    /// JSON Schema describing the output payload.
    pub output_schema: serde_json::Value,
    /// Who provides this tool.
    pub provider: ToolProvider,
}

/// A named thread of conversation between one or more agents.
///
/// Messages are NOT stored inline; retrieve them from the chat manager
/// using [`ConversationId`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    /// Stable identity for this conversation.
    pub id: ConversationId,
    /// Optional display name (e.g. `"planning session"`).
    pub name: Option<String>,
    /// Agents participating in this conversation.
    pub participants: HashSet<AgentId>,
    /// Wall-clock time at which the conversation was created.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_id_new_produces_unique_ids() {
        let a = AgentId::new();
        let b = AgentId::new();
        assert_ne!(a, b, "two freshly generated AgentIds must not be equal");
    }

    #[test]
    fn conversation_id_new_produces_unique_ids() {
        let a = ConversationId::new();
        let b = ConversationId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn message_id_new_produces_unique_ids() {
        let a = MessageId::new();
        let b = MessageId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn tool_id_new_produces_unique_ids() {
        let a = ToolId::new();
        let b = ToolId::new();
        assert_ne!(a, b, "two freshly generated ToolIds must not be equal");
    }

    #[test]
    fn tool_id_display_is_valid_uuid_string() {
        let id = ToolId::new();
        let s = id.to_string();
        assert_eq!(s.len(), 36, "display should be a hyphenated UUID: {s}");
        assert!(
            s.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "display must only contain hex digits and hyphens: {s}"
        );
    }

    #[test]
    fn agent_id_display_is_valid_uuid_string() {
        let id = AgentId::new();
        let s = id.to_string();
        // A UUID v4 hyphenated string is always 36 characters.
        assert_eq!(s.len(), 36, "display should be a hyphenated UUID: {s}");
        assert!(
            s.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
            "display must only contain hex digits and hyphens: {s}"
        );
    }

    #[test]
    fn agent_info_round_trips_through_json() {
        let original = AgentInfo {
            id: AgentId::new(),
            name: "claude-code-1".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        };
        let json = serde_json::to_string(&original).expect("serialization must succeed");
        let recovered: AgentInfo =
            serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(original.id, recovered.id, "id must survive round-trip");
        assert_eq!(
            original.name, recovered.name,
            "name must survive round-trip"
        );
        assert_eq!(
            original.status, recovered.status,
            "status must survive round-trip"
        );
    }

    #[test]
    fn agent_status_serializes_as_snake_case() {
        let connected =
            serde_json::to_string(&AgentStatus::Connected).expect("serialization must succeed");
        assert_eq!(connected, r#""connected""#);

        let disconnected =
            serde_json::to_string(&AgentStatus::Disconnected).expect("serialization must succeed");
        assert_eq!(disconnected, r#""disconnected""#);
    }

    #[test]
    fn chat_message_round_trips_through_json() {
        let conv_id = ConversationId::new();
        let sender = AgentId::new();
        let original = ChatMessage {
            id: MessageId::new(),
            conversation_id: conv_id,
            sender,
            content: "hello world".to_owned(),
            timestamp: Utc::now(),
        };
        let json = serde_json::to_string(&original).expect("serialization must succeed");
        let recovered: ChatMessage =
            serde_json::from_str(&json).expect("deserialization must succeed");

        assert_eq!(original.id, recovered.id, "id must survive round-trip");
        assert_eq!(
            original.conversation_id, recovered.conversation_id,
            "conversation_id must survive round-trip"
        );
        assert_eq!(
            original.sender, recovered.sender,
            "sender must survive round-trip"
        );
        assert_eq!(
            original.content, recovered.content,
            "content must survive round-trip"
        );
    }
}
