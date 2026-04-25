//! Broadcast message bus for intra-process event propagation.
//!
//! [`MessageBus`] wraps a [`tokio::sync::broadcast`] channel and provides
//! typed [`Event`] delivery to any number of concurrent subscribers.

use serde::Serialize;
use tokio::sync::broadcast;

use crate::core::types::{AgentId, AgentInfo, ChatMessage, SkillId, SkillInfo};

/// Events propagated through the message bus.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A new agent has registered with the nexus.
    AgentRegistered(AgentInfo),
    /// An agent has been removed from the registry.
    AgentDeregistered(AgentId),
    /// A new message has been posted to a conversation.
    NewMessage(ChatMessage),
    /// A new skill has been registered.
    SkillRegistered(SkillInfo),
    /// A skill has been deregistered.
    SkillDeregistered(SkillId),
}

/// Internal broadcast channel for intra-process event propagation.
///
/// All events are sent once and fanned out to every active subscriber.
/// Subscribers that fall behind by more than `capacity` events will receive
/// a [`broadcast::error::RecvError::Lagged`] error on their next receive.
pub struct MessageBus {
    sender: broadcast::Sender<Event>,
}

impl MessageBus {
    /// Create a new [`MessageBus`] with the given channel `capacity`.
    ///
    /// `capacity` is the maximum number of events buffered before the
    /// oldest event is overwritten for lagging receivers.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Publish an [`Event`] to all current subscribers.
    ///
    /// If there are no active subscribers the event is silently discarded.
    pub fn publish(&self, event: Event) {
        // Discard the send-error: it just means there are no receivers.
        let _ = self.sender.send(event);
    }

    /// Subscribe to future events.
    ///
    /// The returned [`broadcast::Receiver`] will only receive events
    /// published **after** this call returns.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::core::types::{AgentId, AgentInfo, AgentStatus, ConversationId, MessageId};

    fn make_agent_info() -> AgentInfo {
        AgentInfo {
            id: AgentId::new(),
            name: "test-agent".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        }
    }

    fn make_message() -> ChatMessage {
        ChatMessage {
            id: MessageId::new(),
            conversation_id: ConversationId::new(),
            sender: AgentId::new(),
            content: "hello".to_owned(),
            timestamp: Utc::now(),
        }
    }

    #[tokio::test]
    async fn publish_and_subscribe_should_deliver_event_to_subscriber() {
        let bus = MessageBus::new(16);
        let mut rx = bus.subscribe();

        let info = make_agent_info();
        bus.publish(Event::AgentRegistered(info.clone()));

        let received = rx.recv().await.expect("subscriber must receive the event");
        let Event::AgentRegistered(received_info) = received else {
            panic!("expected AgentRegistered, got something else");
        };
        assert_eq!(
            received_info.id, info.id,
            "received agent id must match published agent id"
        );
    }

    #[tokio::test]
    async fn multiple_subscribers_should_each_receive_same_event() {
        let bus = MessageBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        let msg = make_message();
        bus.publish(Event::NewMessage(msg.clone()));

        let ev1 = rx1
            .recv()
            .await
            .expect("subscriber 1 must receive the event");
        let ev2 = rx2
            .recv()
            .await
            .expect("subscriber 2 must receive the event");

        let Event::NewMessage(m1) = ev1 else {
            panic!("subscriber 1: expected NewMessage");
        };
        let Event::NewMessage(m2) = ev2 else {
            panic!("subscriber 2: expected NewMessage");
        };

        assert_eq!(m1.id, msg.id, "subscriber 1 message id must match");
        assert_eq!(m2.id, msg.id, "subscriber 2 message id must match");
    }

    #[tokio::test]
    async fn subscriber_after_publish_should_miss_earlier_event() {
        let bus = MessageBus::new(16);

        // Publish before subscribing.
        bus.publish(Event::AgentDeregistered(AgentId::new()));

        let mut rx = bus.subscribe();

        // Nothing should be waiting for this late subscriber.
        let result = tokio::time::timeout(std::time::Duration::from_millis(20), rx.recv()).await;

        assert!(
            result.is_err(),
            "late subscriber must not receive events published before it subscribed"
        );
    }

    #[tokio::test]
    async fn publish_with_no_subscribers_should_not_panic() {
        let bus = MessageBus::new(16);
        // No subscribers — must complete without panicking.
        bus.publish(Event::AgentRegistered(make_agent_info()));
        bus.publish(Event::NewMessage(make_message()));
    }
}
