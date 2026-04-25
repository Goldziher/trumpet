//! Task router — selects which agent should handle a given task.
//!
//! The [`TaskRouter`] trait is object-safe so that alternative routing
//! strategies can be swapped in without touching the server layer.

use crate::core::task_types::Task;
use crate::core::types::{AgentId, AgentInfo, AgentStatus};

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Selects an agent to handle a [`Task`] from a slice of candidate agents.
pub trait TaskRouter: Send + Sync {
    /// Return the [`AgentId`] of the selected agent, or `None` when no
    /// suitable agent is available.
    fn select_agent(&self, task: &Task, agents: &[&AgentInfo]) -> Option<AgentId>;
}

// ── DefaultTaskRouter ─────────────────────────────────────────────────────────

/// Default routing strategy used by the nexus.
///
/// Selection order:
///
/// 1. **Explicit assignment** — if the task's `assignee` is connected, return
///    it immediately.
/// 2. **First connected agent** — fall back to the first
///    [`AgentStatus::Connected`] agent in the slice (capability-based matching
///    will be added later, see ADR-013).
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultTaskRouter;

impl TaskRouter for DefaultTaskRouter {
    fn select_agent(&self, task: &Task, agents: &[&AgentInfo]) -> Option<AgentId> {
        // 1. Honour an explicit assignment when the assignee is connected.
        if let Some(assignee) = task.assignee
            && agents
                .iter()
                .any(|a| a.id == assignee && a.status == AgentStatus::Connected)
        {
            return Some(assignee);
        }

        // 2. First connected agent (capability matching deferred to ADR-013).
        agents
            .iter()
            .find(|a| a.status == AgentStatus::Connected)
            .map(|a| a.id)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;

    use super::*;
    use crate::core::bus::MessageBus;
    use crate::core::task_manager::TaskManager;
    use crate::core::task_types::{MessageRole, Part, TaskMessage};
    use crate::core::types::MessageId;

    fn make_agent(id: AgentId, status: AgentStatus) -> AgentInfo {
        AgentInfo {
            id,
            name: "test-agent".to_owned(),
            registered_at: Utc::now(),
            status,
            capabilities: None,
        }
    }

    fn make_task(assignee: Option<AgentId>) -> Task {
        let bus = Arc::new(MessageBus::new(16));
        let mut mgr = TaskManager::new(bus);
        mgr.create_task(
            TaskMessage {
                id: MessageId::new(),
                role: MessageRole::User,
                parts: vec![Part::Text {
                    text: "do something".to_owned(),
                }],
                metadata: None,
            },
            None,
            assignee,
            None,
            None,
        )
        .expect("create_task must succeed in test helper")
    }

    #[test]
    fn explicit_assignment_returns_assignee() {
        let id = AgentId::new();
        let agent = make_agent(id, AgentStatus::Connected);
        let task = make_task(Some(id));
        let router = DefaultTaskRouter;

        let selected = router.select_agent(&task, &[&agent]);

        assert_eq!(
            selected,
            Some(id),
            "should return the explicitly assigned connected agent"
        );
    }

    #[test]
    fn explicit_assignment_skips_disconnected() {
        let id = AgentId::new();
        let agent = make_agent(id, AgentStatus::Disconnected);
        let task = make_task(Some(id));
        let router = DefaultTaskRouter;

        let selected = router.select_agent(&task, &[&agent]);

        assert_eq!(
            selected, None,
            "should return None when assignee is Disconnected and no other agents available"
        );
    }

    #[test]
    fn falls_back_to_connected_agent() {
        let id = AgentId::new();
        let connected = make_agent(id, AgentStatus::Connected);
        let task = make_task(None);
        let router = DefaultTaskRouter;

        let selected = router.select_agent(&task, &[&connected]);

        assert_eq!(
            selected,
            Some(id),
            "should fall back to the first connected agent when no assignee"
        );
    }
}
