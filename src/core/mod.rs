//! Core domain types and logic for the Trumpet agent nexus.
//!
//! This module is transport-agnostic and has no dependencies on any other
//! `src/` module besides `error`. All types and managers here are used
//! throughout the system.

pub mod bus;
pub mod chat;
pub mod registry;
pub mod skills;
pub mod types;

pub use bus::{Event, MessageBus};
pub use chat::ChatManager;
pub use registry::AgentRegistry;
pub use skills::SkillRegistry;
pub use types::{
    AgentId, AgentInfo, AgentStatus, ChatMessage, Conversation, ConversationId, MessageId, SkillId,
    SkillInfo, SkillProvider,
};
