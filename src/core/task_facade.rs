//! Unified task operations facade (ADR-014).
//!
//! [`TaskFacade`] is the single entry point for all task operations across
//! every transport adapter (MCP, gRPC, REST). It orchestrates validation,
//! state transitions, routing, and bus event publishing.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::core::registry::AgentRegistry;
use crate::core::router::TaskRouter;
use crate::core::task_manager::{TaskEvent, TaskManager};
use crate::core::task_types::{
    Artifact, ContextId, Task, TaskFilter, TaskId, TaskMessage, TaskState,
};
use crate::core::types::AgentId;
use crate::error::Error;

/// Canonical task API used by all transport adapters.
///
/// Every protocol adapter (gRPC, MCP, REST) calls through the facade.
/// Protocol-specific type conversion happens at the adapter boundary.
pub struct TaskFacade {
    tasks: Arc<RwLock<TaskManager>>,
    registry: Arc<RwLock<AgentRegistry>>,
    router: Box<dyn TaskRouter>,
}

impl TaskFacade {
    /// Create a new facade backed by the given task manager, agent registry,
    /// and router.
    pub fn new(
        tasks: Arc<RwLock<TaskManager>>,
        registry: Arc<RwLock<AgentRegistry>>,
        router: Box<dyn TaskRouter>,
    ) -> Self {
        Self {
            tasks,
            registry,
            router,
        }
    }

    /// Submit a new task. Routes to an available agent if possible.
    pub async fn submit_task(
        &self,
        message: TaskMessage,
        context_id: Option<ContextId>,
        assignee: Option<AgentId>,
        metadata: Option<serde_json::Value>,
    ) -> Result<Task, Error> {
        let task = {
            let mut mgr = self.tasks.write().await;
            mgr.create_task(message, context_id, assignee, None, metadata)?
        };

        // Attempt routing — read registry without holding the task write lock.
        let registry = self.registry.read().await;
        let agents = registry.list();
        if let Some(agent_id) = self.router.select_agent(&task, &agents) {
            drop(registry);
            let mut mgr = self.tasks.write().await;
            if let Some(t) = mgr.get_mut(&task.id) {
                t.assignee = Some(agent_id);
                return Ok(t.clone());
            }
        }

        Ok(task)
    }

    /// Transition a task to a new state.
    pub async fn update_status(
        &self,
        task_id: &TaskId,
        new_state: TaskState,
        message: Option<TaskMessage>,
    ) -> Result<Task, Error> {
        let mut mgr = self.tasks.write().await;
        mgr.update_status(task_id, new_state, message)
    }

    /// Append an artifact to a task.
    pub async fn add_artifact(&self, task_id: &TaskId, artifact: Artifact) -> Result<Task, Error> {
        let mut mgr = self.tasks.write().await;
        mgr.add_artifact(task_id, artifact)
    }

    /// Cancel a task.
    pub async fn cancel_task(
        &self,
        task_id: &TaskId,
        message: Option<TaskMessage>,
    ) -> Result<Task, Error> {
        let mut mgr = self.tasks.write().await;
        mgr.cancel(task_id, message)
    }

    /// Get a task by ID.
    pub async fn get_task(&self, task_id: &TaskId) -> Result<Task, Error> {
        let mgr = self.tasks.read().await;
        mgr.get(task_id).cloned().ok_or(Error::TaskNotFound {
            id: task_id.to_string(),
        })
    }

    /// List tasks matching a filter.
    pub async fn list_tasks(&self, filter: &TaskFilter) -> Vec<Task> {
        let mgr = self.tasks.read().await;
        mgr.list_filtered(filter).into_iter().cloned().collect()
    }

    /// Subscribe to task-lifecycle events.
    pub async fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TaskEvent> {
        let mgr = self.tasks.read().await;
        mgr.subscribe()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::core::bus::MessageBus;
    use crate::core::registry::AgentRegistry;
    use crate::core::router::DefaultTaskRouter;
    use crate::core::task_types::{MessageRole, Part};
    use crate::core::types::MessageId;

    fn make_facade() -> TaskFacade {
        let bus = Arc::new(MessageBus::new(64));
        TaskFacade::new(
            Arc::new(RwLock::new(TaskManager::new(Arc::clone(&bus)))),
            Arc::new(RwLock::new(AgentRegistry::new(Arc::clone(&bus)))),
            Box::new(DefaultTaskRouter),
        )
    }

    fn make_message() -> TaskMessage {
        TaskMessage {
            id: MessageId::new(),
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: "do something".to_owned(),
            }],
            metadata: None,
        }
    }

    #[tokio::test]
    async fn submit_task_creates_in_submitted_state() {
        let facade = make_facade();
        let task = facade
            .submit_task(make_message(), None, None, None)
            .await
            .expect("submit must succeed");

        assert_eq!(task.status.state, TaskState::Submitted);
    }

    #[tokio::test]
    async fn submit_then_get_returns_same_task() {
        let facade = make_facade();
        let task = facade
            .submit_task(make_message(), None, None, None)
            .await
            .unwrap();

        let fetched = facade.get_task(&task.id).await.unwrap();
        assert_eq!(fetched.id, task.id);
    }

    #[tokio::test]
    async fn update_status_transitions_task() {
        let facade = make_facade();
        let task = facade
            .submit_task(make_message(), None, None, None)
            .await
            .unwrap();

        let updated = facade
            .update_status(&task.id, TaskState::Working, None)
            .await
            .unwrap();
        assert_eq!(updated.status.state, TaskState::Working);
    }

    #[tokio::test]
    async fn cancel_task_from_submitted() {
        let facade = make_facade();
        let task = facade
            .submit_task(make_message(), None, None, None)
            .await
            .unwrap();

        let canceled = facade.cancel_task(&task.id, None).await.unwrap();
        assert_eq!(canceled.status.state, TaskState::Canceled);
    }

    #[tokio::test]
    async fn list_tasks_empty_initially() {
        let facade = make_facade();
        let tasks = facade.list_tasks(&TaskFilter::default()).await;
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn submit_routes_to_connected_agent() {
        let bus = Arc::new(MessageBus::new(64));
        let registry = Arc::new(RwLock::new(AgentRegistry::new(Arc::clone(&bus))));

        // Register a connected agent.
        let agent = {
            let mut reg = registry.write().await;
            reg.register("worker").unwrap()
        };

        let facade = TaskFacade::new(
            Arc::new(RwLock::new(TaskManager::new(Arc::clone(&bus)))),
            Arc::clone(&registry),
            Box::new(DefaultTaskRouter),
        );

        let task = facade
            .submit_task(make_message(), None, None, None)
            .await
            .unwrap();

        assert_eq!(
            task.assignee,
            Some(agent.id),
            "task should be routed to the connected agent"
        );
    }
}
