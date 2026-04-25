//! Core domain types and logic for the Trumpet agent nexus.
//!
//! This module is transport-agnostic and has no dependencies on any other
//! `src/` module besides `error`. All types and managers here are used
//! throughout the system.

pub mod bus;
pub mod chat;
pub mod code_tools;
pub mod registry;
pub mod router;
pub mod skills;
pub mod task_manager;
pub mod task_types;
pub mod types;

pub use bus::{Event, MessageBus};
pub use chat::ChatManager;
pub use registry::AgentRegistry;
pub use router::{DefaultTaskRouter, TaskRouter};
pub use skills::SkillRegistry;
pub use task_manager::{TaskEvent, TaskManager};
pub use task_types::{
    AgentCapabilities, Artifact, ArtifactId, ContextId, MessageRole, Part, Task, TaskFilter,
    TaskId, TaskMessage, TaskState, TaskStatus,
};
pub use types::{
    AgentId, AgentInfo, AgentStatus, ChatMessage, Conversation, ConversationId, MessageId, SkillId,
    SkillInfo, SkillProvider,
};
