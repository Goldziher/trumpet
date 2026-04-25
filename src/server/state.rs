//! Shared application state threaded through all axum handlers.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::Config;
use crate::core::{AgentRegistry, ChatManager, MessageBus, SkillRegistry};

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
    /// Broadcast bus used for SSE event delivery.
    pub bus: Arc<MessageBus>,
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
            bus,
            config: Arc::new(config),
        }
    }
}
