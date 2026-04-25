//! Conversation manager for the Trumpet agent nexus.
//!
//! [`ChatManager`] owns all in-memory conversation state and message history,
//! and publishes [`Event::NewMessage`] on every successful send so that any
//! subscriber can react without polling.
//!
//! Messages are stored separately from [`Conversation`] so that listing all
//! conversations remains a cheap pointer-walk rather than a heap traversal.

use std::sync::Arc;

use ahash::AHashMap;
use chrono::Utc;

use crate::core::bus::{Event, MessageBus};
use crate::core::types::{AgentId, ChatMessage, Conversation, ConversationId, MessageId};
use crate::error::Error;

/// Manages conversations and their message histories.
///
/// All state is in-memory and not persisted; persistence is the responsibility
/// of a higher-level state manager layer.
pub struct ChatManager {
    conversations: AHashMap<ConversationId, Conversation>,
    messages: AHashMap<ConversationId, Vec<ChatMessage>>,
    bus: Arc<MessageBus>,
}

impl ChatManager {
    /// Create a new [`ChatManager`] backed by `bus` for event publishing.
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            conversations: AHashMap::new(),
            messages: AHashMap::new(),
            bus,
        }
    }

    /// Create a new conversation with an optional display `name` and a set of
    /// `participants`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConversationEmptyParticipants`] when `participants` is
    /// empty.
    pub fn create_conversation(
        &mut self,
        name: Option<String>,
        participants: Vec<AgentId>,
    ) -> Result<Conversation, Error> {
        if participants.is_empty() {
            return Err(Error::ConversationEmptyParticipants);
        }

        let conv = Conversation {
            id: ConversationId::new(),
            name,
            participants: participants.into_iter().collect(),
            created_at: Utc::now(),
        };

        self.conversations.insert(conv.id, conv.clone());
        self.messages.insert(conv.id, Vec::new());

        Ok(conv)
    }

    /// Send a message from `sender` into `conversation_id`.
    ///
    /// On success the message is stored and a [`Event::NewMessage`] event is
    /// published to the bus.
    ///
    /// # Errors
    ///
    /// - [`Error::ConversationNotFound`] — no conversation with that ID exists.
    /// - [`Error::ConversationNotParticipant`] — `sender` is not a participant.
    pub fn send_message(
        &mut self,
        conversation_id: &ConversationId,
        sender: AgentId,
        content: String,
    ) -> Result<ChatMessage, Error> {
        let conv =
            self.conversations
                .get(conversation_id)
                .ok_or_else(|| Error::ConversationNotFound {
                    id: conversation_id.to_string(),
                })?;

        if !conv.participants.contains(&sender) {
            return Err(Error::ConversationNotParticipant {
                agent: sender.to_string(),
                id: conversation_id.to_string(),
            });
        }

        let msg = ChatMessage {
            id: MessageId::new(),
            conversation_id: *conversation_id,
            sender,
            content,
            timestamp: Utc::now(),
        };

        self.messages
            .entry(*conversation_id)
            .or_default()
            .push(msg.clone());

        self.bus.publish(Event::NewMessage(msg.clone()));

        Ok(msg)
    }

    /// Return a reference to the conversation with `id`, or `None`.
    pub fn get_conversation(&self, id: &ConversationId) -> Option<&Conversation> {
        self.conversations.get(id)
    }

    /// Return the ordered message history for a conversation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConversationNotFound`] when no conversation with that
    /// ID exists.
    pub fn get_messages(&self, conversation_id: &ConversationId) -> Result<&[ChatMessage], Error> {
        self.messages
            .get(conversation_id)
            .map(Vec::as_slice)
            .ok_or_else(|| Error::ConversationNotFound {
                id: conversation_id.to_string(),
            })
    }

    /// Return references to all known conversations in unspecified order.
    pub fn list_conversations(&self) -> Vec<&Conversation> {
        self.conversations.values().collect()
    }

    /// Return all conversations that `agent_id` participates in.
    pub fn conversations_for_agent(&self, agent_id: &AgentId) -> Vec<&Conversation> {
        self.conversations
            .values()
            .filter(|conv| conv.participants.contains(agent_id))
            .collect()
    }

    /// Clear all conversations and messages, then repopulate.
    ///
    /// Used during daemon startup to restore persisted state. No bus events
    /// are published.
    pub fn restore(
        &mut self,
        conversations: Vec<Conversation>,
        messages: Vec<(ConversationId, Vec<ChatMessage>)>,
    ) {
        self.conversations.clear();
        self.messages.clear();
        for conv in conversations {
            self.conversations.insert(conv.id, conv);
        }
        for (conv_id, msgs) in messages {
            self.messages.insert(conv_id, msgs);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::bus::Event;

    fn make_bus() -> Arc<MessageBus> {
        Arc::new(MessageBus::new(64))
    }

    fn make_agent() -> AgentId {
        AgentId::new()
    }

    // ── create_conversation ───────────────────────────────────────────────────

    #[tokio::test]
    async fn create_conversation_succeeds() {
        let mut mgr = ChatManager::new(make_bus());
        let agent = make_agent();

        let conv = mgr
            .create_conversation(Some("planning".to_owned()), vec![agent])
            .expect("create_conversation must succeed with one participant");

        assert_eq!(
            conv.name.as_deref(),
            Some("planning"),
            "conversation name must match"
        );
        assert_eq!(conv.participants.len(), 1, "participants count must match");
        assert!(
            conv.participants.contains(&agent),
            "participant must be present"
        );
        assert!(
            mgr.get_conversation(&conv.id).is_some(),
            "conversation must be retrievable by id"
        );
    }

    #[tokio::test]
    async fn create_conversation_empty_participants_fails() {
        let mut mgr = ChatManager::new(make_bus());

        let err = mgr
            .create_conversation(None, vec![])
            .expect_err("empty participants must return an error");

        assert!(
            matches!(err, Error::ConversationEmptyParticipants),
            "expected ConversationEmptyParticipants, got: {err:?}"
        );
    }

    // ── send_message ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn send_message_succeeds() {
        let mut mgr = ChatManager::new(make_bus());
        let agent = make_agent();

        let conv = mgr
            .create_conversation(None, vec![agent])
            .expect("create must succeed");

        let msg = mgr
            .send_message(&conv.id, agent, "hello".to_owned())
            .expect("send_message must succeed for a participant");

        assert_eq!(msg.sender, agent, "sender must match");
        assert_eq!(msg.content, "hello", "content must match");
        assert_eq!(msg.conversation_id, conv.id, "conversation_id must match");

        let messages = mgr
            .get_messages(&conv.id)
            .expect("get_messages must succeed");
        assert_eq!(messages.len(), 1, "one message must be stored");
        assert_eq!(messages[0].id, msg.id, "stored message id must match");
    }

    #[tokio::test]
    async fn send_message_unknown_conversation_fails() {
        let mut mgr = ChatManager::new(make_bus());
        let agent = make_agent();
        let unknown_id = ConversationId::new();

        let err = mgr
            .send_message(&unknown_id, agent, "hi".to_owned())
            .expect_err("send to unknown conversation must fail");

        assert!(
            matches!(err, Error::ConversationNotFound { .. }),
            "expected ConversationNotFound, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn send_message_non_participant_fails() {
        let mut mgr = ChatManager::new(make_bus());
        let participant = make_agent();
        let outsider = make_agent();

        let conv = mgr
            .create_conversation(None, vec![participant])
            .expect("create must succeed");

        let err = mgr
            .send_message(&conv.id, outsider, "hi".to_owned())
            .expect_err("non-participant send must fail");

        assert!(
            matches!(err, Error::ConversationNotParticipant { .. }),
            "expected ConversationNotParticipant, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn send_message_publishes_event() {
        let bus = make_bus();
        let mut mgr = ChatManager::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        let agent = make_agent();
        let conv = mgr
            .create_conversation(None, vec![agent])
            .expect("create must succeed");

        let sent = mgr
            .send_message(&conv.id, agent, "event check".to_owned())
            .expect("send must succeed");

        let received = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("event must arrive within timeout")
            .expect("receiver must not fail");

        let Event::NewMessage(event_msg) = received else {
            panic!("expected NewMessage event, got something else");
        };

        assert_eq!(
            event_msg.id, sent.id,
            "published message id must match the sent message"
        );
        assert_eq!(
            event_msg.content, "event check",
            "published content must match"
        );
    }

    // ── conversations_for_agent ───────────────────────────────────────────────

    #[tokio::test]
    async fn conversations_for_agent_filters_correctly() {
        let mut mgr = ChatManager::new(make_bus());
        let agent_a = make_agent();
        let agent_b = make_agent();

        let conv_a = mgr
            .create_conversation(Some("conv-a".to_owned()), vec![agent_a])
            .expect("create conv-a must succeed");

        let _conv_b = mgr
            .create_conversation(Some("conv-b".to_owned()), vec![agent_b])
            .expect("create conv-b must succeed");

        let results = mgr.conversations_for_agent(&agent_a);

        assert_eq!(
            results.len(),
            1,
            "agent_a must appear in exactly one conversation"
        );
        assert_eq!(
            results[0].id, conv_a.id,
            "agent_a's conversation must be conv-a"
        );
    }

    // ── restore ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn restore_populates_conversations_and_messages() {
        use std::collections::HashSet;

        let mut mgr = ChatManager::new(make_bus());
        let agent = make_agent();
        let conv_id = ConversationId::new();
        let msg_id = MessageId::new();

        let conv = Conversation {
            id: conv_id,
            name: Some("restored".to_owned()),
            participants: HashSet::from([agent]),
            created_at: chrono::Utc::now(),
        };
        let msg = ChatMessage {
            id: msg_id,
            conversation_id: conv_id,
            sender: agent,
            content: "restored message".to_owned(),
            timestamp: chrono::Utc::now(),
        };

        mgr.restore(vec![conv], vec![(conv_id, vec![msg])]);

        let found = mgr
            .get_conversation(&conv_id)
            .expect("conversation must exist");
        assert_eq!(found.id, conv_id);
        assert_eq!(found.name.as_deref(), Some("restored"));

        let msgs = mgr.get_messages(&conv_id).expect("messages must exist");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].id, msg_id);
        assert_eq!(msgs[0].content, "restored message");
    }

    // ── get_messages ordering ─────────────────────────────────────────────────

    #[tokio::test]
    async fn get_messages_returns_ordered_messages() {
        let mut mgr = ChatManager::new(make_bus());
        let agent = make_agent();

        let conv = mgr
            .create_conversation(None, vec![agent])
            .expect("create must succeed");

        let first = mgr
            .send_message(&conv.id, agent, "first".to_owned())
            .expect("first send must succeed");

        let second = mgr
            .send_message(&conv.id, agent, "second".to_owned())
            .expect("second send must succeed");

        let messages = mgr
            .get_messages(&conv.id)
            .expect("get_messages must succeed");

        assert_eq!(messages.len(), 2, "two messages must be stored");
        assert_eq!(messages[0].id, first.id, "first message must be at index 0");
        assert_eq!(
            messages[1].id, second.id,
            "second message must be at index 1"
        );
        assert_eq!(messages[0].content, "first", "first content must match");
        assert_eq!(messages[1].content, "second", "second content must match");
    }
}
