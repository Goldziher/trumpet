//! Core domain types and logic for the Trumpet agent nexus.
//!
//! This module is transport-agnostic and has no dependencies on any other
//! `src/` module besides `error`. All types and managers here are used
//! throughout the system.

pub mod bus;
pub mod chat;
pub mod code_tools;
pub mod push_notifications;
pub mod registry;
pub mod router;
pub mod task_facade;
pub mod task_manager;
pub mod task_types;
pub mod tool_invoker;
pub mod tools;
pub mod types;

pub use bus::{Event, MessageBus};
pub use chat::ChatManager;
pub use push_notifications::{
    PushNotificationAuth, PushNotificationConfig, PushNotificationId, PushNotificationStore,
};
pub use registry::AgentRegistry;
pub use router::{DefaultTaskRouter, TaskRouter};
pub use task_facade::TaskFacade;
pub use task_manager::{TaskEvent, TaskManager};
pub use task_types::{
    AgentCapabilities, Artifact, ArtifactId, ContextId, MessageRole, Part, Task, TaskFilter,
    TaskId, TaskMessage, TaskState, TaskStatus,
};
pub use tool_invoker::ToolInvoker;
pub use tools::{ToolRegistry, ToolResult};
pub use types::{
    AgentId, AgentInfo, AgentStatus, ChatMessage, Conversation, ConversationId, MessageId, ToolId,
    ToolInfo, ToolProvider,
};
