//! Serializable snapshot of all in-memory daemon state.
//!
//! A [`StateSnapshot`] captures agents, skills, conversations, and messages
//! at a point in time. It is serialized as MessagePack and written to `snapshot.msgpack` by [`super::StateManager`].

use serde::{Deserialize, Serialize};

use crate::core::types::{AgentInfo, ChatMessage, Conversation, ConversationId, SkillInfo};

/// A full, point-in-time snapshot of all persistent daemon state.
///
/// All collections are owned `Vec` values so the snapshot can be serialized
/// and moved independently of the in-memory registries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// All registered agents at the time of the snapshot.
    pub agents: Vec<AgentInfo>,
    /// All registered skills at the time of the snapshot.
    pub skills: Vec<SkillInfo>,
    /// All known conversation metadata at the time of the snapshot.
    pub conversations: Vec<Conversation>,
    /// Message histories keyed by conversation ID, in insertion order.
    pub messages: Vec<(ConversationId, Vec<ChatMessage>)>,
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::core::types::{AgentId, AgentStatus, MessageId, SkillId, SkillProvider};

    fn make_agent_info(name: &str) -> AgentInfo {
        AgentInfo {
            id: AgentId::new(),
            name: name.to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        }
    }

    fn make_skill_info(name: &str) -> SkillInfo {
        SkillInfo {
            id: SkillId::new(),
            name: name.to_owned(),
            description: format!("{name} description"),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            provider: SkillProvider::BuiltIn,
        }
    }

    fn make_conversation(participants: Vec<AgentId>) -> Conversation {
        use std::collections::HashSet;
        Conversation {
            id: ConversationId::new(),
            name: Some("test-conv".to_owned()),
            participants: HashSet::from_iter(participants),
            created_at: Utc::now(),
        }
    }

    fn make_message(conv_id: ConversationId, sender: AgentId, content: &str) -> ChatMessage {
        ChatMessage {
            id: MessageId::new(),
            conversation_id: conv_id,
            sender,
            content: content.to_owned(),
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn snapshot_round_trip_via_msgpack() {
        let agent1 = make_agent_info("agent-one");
        let agent2 = make_agent_info("agent-two");
        let skill1 = make_skill_info("skill-one");
        let skill2 = make_skill_info("skill-two");
        let conv = make_conversation(vec![agent1.id, agent2.id]);
        let msg1 = make_message(conv.id, agent1.id, "hello");
        let msg2 = make_message(conv.id, agent2.id, "world");

        let original = StateSnapshot {
            agents: vec![agent1.clone(), agent2.clone()],
            skills: vec![skill1.clone(), skill2.clone()],
            conversations: vec![conv.clone()],
            messages: vec![(conv.id, vec![msg1.clone(), msg2.clone()])],
        };

        let encoded = rmp_serde::to_vec(&original).expect("MessagePack encoding must succeed");

        let decoded: StateSnapshot =
            rmp_serde::from_slice(&encoded).expect("MessagePack decoding must succeed");

        assert_eq!(
            decoded.agents.len(),
            2,
            "decoded snapshot must contain 2 agents"
        );
        assert_eq!(
            decoded.skills.len(),
            2,
            "decoded snapshot must contain 2 skills"
        );
        assert_eq!(
            decoded.conversations.len(),
            1,
            "decoded snapshot must contain 1 conversation"
        );
        assert_eq!(
            decoded.messages.len(),
            1,
            "decoded snapshot must contain message history for 1 conversation"
        );

        // Verify agent identity survives the round-trip.
        let decoded_agent_ids: Vec<_> = decoded.agents.iter().map(|a| a.id).collect();
        assert!(
            decoded_agent_ids.contains(&agent1.id),
            "agent-one id must survive round-trip"
        );
        assert!(
            decoded_agent_ids.contains(&agent2.id),
            "agent-two id must survive round-trip"
        );

        // Verify agent names survive the round-trip.
        let decoded_names: Vec<_> = decoded.agents.iter().map(|a| a.name.as_str()).collect();
        assert!(
            decoded_names.contains(&"agent-one"),
            "agent-one name must survive round-trip"
        );
        assert!(
            decoded_names.contains(&"agent-two"),
            "agent-two name must survive round-trip"
        );

        // Verify skill identity.
        let decoded_skill_ids: Vec<_> = decoded.skills.iter().map(|s| s.id).collect();
        assert!(
            decoded_skill_ids.contains(&skill1.id),
            "skill-one id must survive round-trip"
        );

        // Verify conversation.
        assert_eq!(
            decoded.conversations[0].id, conv.id,
            "conversation id must survive round-trip"
        );

        // Verify messages.
        let (decoded_conv_id, decoded_msgs) = &decoded.messages[0];
        assert_eq!(
            *decoded_conv_id, conv.id,
            "message key conversation id must survive round-trip"
        );
        assert_eq!(
            decoded_msgs.len(),
            2,
            "both messages must survive round-trip"
        );
        assert_eq!(
            decoded_msgs[0].id, msg1.id,
            "first message id must survive round-trip"
        );
        assert_eq!(
            decoded_msgs[1].id, msg2.id,
            "second message id must survive round-trip"
        );
        assert_eq!(
            decoded_msgs[0].content, "hello",
            "first message content must survive round-trip"
        );
        assert_eq!(
            decoded_msgs[1].content, "world",
            "second message content must survive round-trip"
        );
    }
}
