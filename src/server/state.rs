//! Shared application state threaded through all axum handlers.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::Config;
use crate::core::code_tools::CodeTools;
use crate::core::{AgentRegistry, ChatManager, MessageBus, SkillRegistry, TaskManager};

/// Cloneable handle to shared daemon state.
///
/// All fields are wrapped in [`Arc`] so that cloning the state for each
/// handler is cheap — only the reference counts are incremented.
#[derive(Clone)]
pub struct AppState {
    /// Registry of all currently connected agents.
    pub registry: Arc<RwLock<AgentRegistry>>,
    /// Conversation manager for all active chats.
    pub chat: Arc<RwLock<ChatManager>>,
    /// Skill registry for all available capabilities.
    pub skills: Arc<RwLock<SkillRegistry>>,
    /// Task manager for A2A task lifecycle.
    pub tasks: Arc<RwLock<TaskManager>>,
    /// Broadcast bus used for SSE event delivery.
    pub bus: Arc<MessageBus>,
    /// Built-in code intelligence tools (populated after startup).
    pub code_tools: Arc<RwLock<Option<CodeTools>>>,
    /// Immutable daemon configuration.
    pub config: Arc<Config>,
}

impl AppState {
    /// Construct a fresh [`AppState`] from a validated [`Config`].
    pub fn new(config: Config) -> Self {
        let bus = Arc::new(MessageBus::new(1024));
        Self {
            registry: Arc::new(RwLock::new(AgentRegistry::new(Arc::clone(&bus)))),
            chat: Arc::new(RwLock::new(ChatManager::new(Arc::clone(&bus)))),
            skills: Arc::new(RwLock::new(SkillRegistry::new(Arc::clone(&bus)))),
            tasks: Arc::new(RwLock::new(TaskManager::new(Arc::clone(&bus)))),
            bus,
            code_tools: Arc::new(RwLock::new(None)),
            config: Arc::new(config),
        }
    }

    /// Collect a point-in-time [`StateSnapshot`] from all in-memory registries.
    pub async fn to_snapshot(&self) -> crate::state::StateSnapshot {
        let registry = self.registry.read().await;
        let chat = self.chat.read().await;
        let skills = self.skills.read().await;
        let task_mgr = self.tasks.read().await;

        let agents = registry.list().into_iter().cloned().collect();
        let skill_list = skills.list().into_iter().cloned().collect();
        let conversations: Vec<_> = chat.list_conversations().into_iter().cloned().collect();
        let messages = conversations
            .iter()
            .filter_map(|conv| {
                chat.get_messages(&conv.id)
                    .ok()
                    .map(|msgs| (conv.id, msgs.to_vec()))
            })
            .collect();
        let tasks = task_mgr.list().into_iter().cloned().collect();

        crate::state::StateSnapshot {
            agents,
            skills: skill_list,
            conversations,
            messages,
            tasks,
        }
    }

    /// Apply a [`StateSnapshot`] to all in-memory registries.
    ///
    /// Existing state is discarded; no bus events are published during restore.
    pub async fn restore_from_snapshot(&self, snapshot: crate::state::StateSnapshot) {
        let mut registry = self.registry.write().await;
        let mut chat = self.chat.write().await;
        let mut skills = self.skills.write().await;
        let mut task_mgr = self.tasks.write().await;

        registry.restore(snapshot.agents);
        skills.restore(snapshot.skills);
        chat.restore(snapshot.conversations, snapshot.messages);
        task_mgr.restore(snapshot.tasks);
    }
}
