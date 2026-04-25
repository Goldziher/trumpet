//! Task manager — owns all in-memory task state and publishes task events.
//!
//! [`TaskManager`] is intentionally not wrapped in `Arc`/`RwLock`; locking
//! belongs at the server layer. All mutation is `&mut self`.

use std::sync::Arc;

use ahash::AHashMap;
use chrono::Utc;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::core::bus::MessageBus;
use crate::core::task_types::{
    Artifact, ArtifactId, ContextId, Task, TaskFilter, TaskId, TaskMessage, TaskState, TaskStatus,
};
use crate::core::types::AgentId;
use crate::error::Error;

// ── Task-scoped events ────────────────────────────────────────────────────────

/// Events scoped to task lifecycle changes.
///
/// These are broadcast on a dedicated channel independent of the global
/// [`MessageBus`] so consumers can subscribe to task events alone.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TaskEvent {
    /// A new task was created.
    TaskCreated(Box<Task>),
    /// A task's state changed.
    TaskStatusChanged {
        task_id: TaskId,
        old_state: TaskState,
        new_state: TaskState,
    },
    /// An artifact was appended to a task.
    TaskArtifactAdded {
        task_id: TaskId,
        artifact_id: ArtifactId,
    },
}

// ── TaskManager ───────────────────────────────────────────────────────────────

/// Manages tasks across the agent nexus.
///
/// Tasks are indexed by [`TaskId`] for O(1) lookup. A secondary
/// `context_index` maps each [`ContextId`] to its member task IDs for
/// context-scoped queries.
pub struct TaskManager {
    tasks: AHashMap<TaskId, Task>,
    context_index: AHashMap<ContextId, Vec<TaskId>>,
    /// Global message bus — held for future cross-domain event publishing.
    _bus: Arc<MessageBus>,
    event_tx: broadcast::Sender<TaskEvent>,
}

impl TaskManager {
    /// Create a new [`TaskManager`] backed by `bus`.
    pub fn new(bus: Arc<MessageBus>) -> Self {
        let (event_tx, _) = broadcast::channel(64);
        Self {
            tasks: AHashMap::new(),
            context_index: AHashMap::new(),
            _bus: bus,
            event_tx,
        }
    }

    /// Subscribe to task-lifecycle events.
    ///
    /// The returned receiver only captures events published **after** this
    /// call returns.
    pub fn subscribe(&self) -> broadcast::Receiver<TaskEvent> {
        self.event_tx.subscribe()
    }

    /// Create a new task from an initial message.
    ///
    /// A fresh [`TaskId`] is always generated. When `context_id` is `None` a
    /// new [`ContextId`] is generated automatically. The task enters the
    /// [`TaskState::Submitted`] state and a [`TaskEvent::TaskCreated`] event
    /// is published.
    pub fn create_task(
        &mut self,
        message: TaskMessage,
        context_id: Option<ContextId>,
        assignee: Option<AgentId>,
        creator: Option<AgentId>,
        metadata: Option<serde_json::Value>,
    ) -> Result<Task, Error> {
        let id = TaskId::new();
        let context_id = context_id.unwrap_or_default();
        let now = Utc::now();

        let task = Task {
            id,
            context_id,
            status: TaskStatus {
                state: TaskState::Submitted,
                message: Some(message.clone()),
                timestamp: now,
            },
            history: vec![message],
            artifacts: Vec::new(),
            assignee,
            creator,
            metadata,
        };

        self.context_index.entry(context_id).or_default().push(id);
        self.tasks.insert(id, task.clone());

        self.publish(TaskEvent::TaskCreated(Box::new(task.clone())));
        Ok(task)
    }

    /// Look up a task by its [`TaskId`].
    pub fn get(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.get(id)
    }

    /// Return all tasks in unspecified order.
    pub fn list(&self) -> Vec<&Task> {
        self.tasks.values().collect()
    }

    /// Return tasks matching `filter`.
    ///
    /// All non-`None` filter fields must match; unset fields are ignored.
    pub fn list_filtered(&self, filter: &TaskFilter) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| {
                filter
                    .context_id
                    .as_ref()
                    .is_none_or(|c| &t.context_id == c)
                    && filter.state.as_ref().is_none_or(|s| &t.status.state == s)
                    && filter
                        .assignee
                        .as_ref()
                        .is_none_or(|a| t.assignee.as_ref() == Some(a))
            })
            .collect()
    }

    /// Transition a task to `new_state`, optionally appending a status message.
    ///
    /// Validates the transition via [`TaskState::can_transition_to`].
    ///
    /// # Errors
    ///
    /// - [`Error::InternalUnexpected`] — task not found or the transition is
    ///   invalid (current state cannot move to `new_state`).
    pub fn update_status(
        &mut self,
        task_id: &TaskId,
        new_state: TaskState,
        message: Option<TaskMessage>,
    ) -> Result<Task, Error> {
        let old_state = self
            .tasks
            .get(task_id)
            .ok_or_else(|| Error::InternalUnexpected {
                reason: format!("task '{task_id}' not found"),
            })?
            .status
            .state;

        if !old_state.can_transition_to(new_state) {
            return Err(Error::InternalUnexpected {
                reason: format!(
                    "invalid task transition: {old_state:?} → {new_state:?} for task '{task_id}'"
                ),
            });
        }

        let now = Utc::now();
        let task = self.tasks.get_mut(task_id).expect("checked above");

        task.status = TaskStatus {
            state: new_state,
            message: message.clone(),
            timestamp: now,
        };

        if let Some(msg) = message {
            task.history.push(msg);
        }

        let task = task.clone();
        self.publish(TaskEvent::TaskStatusChanged {
            task_id: *task_id,
            old_state,
            new_state,
        });

        Ok(task)
    }

    /// Append an artifact to a task.
    ///
    /// # Errors
    ///
    /// - [`Error::InternalUnexpected`] — task not found.
    pub fn add_artifact(&mut self, task_id: &TaskId, artifact: Artifact) -> Result<Task, Error> {
        let artifact_id = artifact.id;
        let task = self
            .tasks
            .get_mut(task_id)
            .ok_or_else(|| Error::InternalUnexpected {
                reason: format!("task '{task_id}' not found"),
            })?;

        task.artifacts.push(artifact);

        let task = task.clone();
        self.publish(TaskEvent::TaskArtifactAdded {
            task_id: *task_id,
            artifact_id,
        });

        Ok(task)
    }

    /// Cancel a task by transitioning it to [`TaskState::Canceled`].
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Self::update_status`].
    pub fn cancel(&mut self, task_id: &TaskId) -> Result<Task, Error> {
        self.update_status(task_id, TaskState::Canceled, None)
    }

    /// Return active (non-terminal) tasks assigned to `agent_id`.
    ///
    /// Terminal states are [`TaskState::Completed`], [`TaskState::Canceled`],
    /// [`TaskState::Failed`], and [`TaskState::Rejected`].
    pub fn tasks_for_agent(&self, agent_id: &AgentId) -> Vec<&Task> {
        self.tasks
            .values()
            .filter(|t| t.assignee.as_ref() == Some(agent_id) && !t.status.state.is_terminal())
            .collect()
    }

    /// Clear all task state and repopulate from `tasks`.
    ///
    /// Used during daemon startup to restore persisted state. No events are
    /// published.
    pub fn restore(&mut self, tasks: Vec<Task>) {
        self.tasks.clear();
        self.context_index.clear();

        for task in tasks {
            self.context_index
                .entry(task.context_id)
                .or_default()
                .push(task.id);
            self.tasks.insert(task.id, task);
        }
    }

    // ── private helpers ───────────────────────────────────────────────────────

    /// Publish a task event; silently drops the error when no receivers are
    /// active.
    fn publish(&self, event: TaskEvent) {
        let _ = self.event_tx.send(event);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::task_types::{MessageRole, Part};
    use crate::core::types::MessageId;

    fn make_manager() -> TaskManager {
        let bus = Arc::new(MessageBus::new(16));
        TaskManager::new(bus)
    }

    fn make_message() -> TaskMessage {
        TaskMessage {
            id: MessageId::new(),
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: "hello".to_owned(),
            }],
            metadata: None,
        }
    }

    fn make_artifact() -> Artifact {
        Artifact {
            id: ArtifactId::new(),
            name: Some("output.txt".to_owned()),
            description: None,
            parts: vec![Part::Text {
                text: "result".to_owned(),
            }],
            metadata: None,
        }
    }

    // ── create_task ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_task_succeeds() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create_task must succeed");

        assert_eq!(
            task.status.state,
            TaskState::Submitted,
            "new task must start in Submitted state"
        );
        assert_eq!(
            task.history.len(),
            1,
            "history must contain the initial message"
        );
    }

    #[tokio::test]
    async fn create_task_generates_context_id_when_none() {
        let mut mgr = make_manager();
        let task1 = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("first create_task must succeed");
        let task2 = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("second create_task must succeed");

        assert_ne!(
            task1.context_id, task2.context_id,
            "each task with no explicit context_id must get a unique one"
        );
    }

    // ── get ───────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_returns_created_task() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create_task must succeed");

        let found = mgr.get(&task.id).expect("get must return the created task");
        assert_eq!(found.id, task.id, "retrieved task id must match");
    }

    // ── list_filtered ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_filtered_by_state() {
        let mut mgr = make_manager();
        let task1 = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("first create must succeed");
        let task2 = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("second create must succeed");

        mgr.update_status(&task2.id, TaskState::Working, None)
            .expect("transition to Working must succeed");

        let filter = TaskFilter {
            state: Some(TaskState::Submitted),
            context_id: None,
            assignee: None,
        };
        let results = mgr.list_filtered(&filter);

        assert_eq!(results.len(), 1, "only one task should be Submitted");
        assert_eq!(results[0].id, task1.id, "the Submitted task must be task1");
    }

    // ── update_status ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn update_status_valid_transition() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create must succeed");

        let updated = mgr
            .update_status(&task.id, TaskState::Working, None)
            .expect("Submitted → Working is a valid transition");

        assert_eq!(
            updated.status.state,
            TaskState::Working,
            "task must be in Working state after transition"
        );
    }

    #[tokio::test]
    async fn update_status_invalid_transition() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create must succeed");

        let err = mgr
            .update_status(&task.id, TaskState::Completed, None)
            .expect_err("Submitted → Completed must be rejected");

        assert!(
            matches!(err, Error::InternalUnexpected { .. }),
            "expected InternalUnexpected for invalid transition, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn update_status_terminal_rejects() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create must succeed");

        mgr.update_status(&task.id, TaskState::Working, None)
            .expect("Submitted → Working");
        mgr.update_status(&task.id, TaskState::Completed, None)
            .expect("Working → Completed");

        let err = mgr
            .update_status(&task.id, TaskState::Working, None)
            .expect_err("Completed → Working must be rejected");

        assert!(
            matches!(err, Error::InternalUnexpected { .. }),
            "expected InternalUnexpected for terminal → active, got: {err:?}"
        );
    }

    // ── add_artifact ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn add_artifact_succeeds() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create must succeed");

        let artifact = make_artifact();
        let artifact_id = artifact.id;

        let updated = mgr
            .add_artifact(&task.id, artifact)
            .expect("add_artifact must succeed");

        assert_eq!(
            updated.artifacts.len(),
            1,
            "task must have exactly one artifact"
        );
        assert_eq!(
            updated.artifacts[0].id, artifact_id,
            "artifact id must match"
        );
    }

    // ── cancel ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_from_working_succeeds() {
        let mut mgr = make_manager();
        let task = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create must succeed");

        mgr.update_status(&task.id, TaskState::Working, None)
            .expect("Submitted → Working");

        let canceled = mgr
            .cancel(&task.id)
            .expect("cancel must succeed from Working");

        assert_eq!(
            canceled.status.state,
            TaskState::Canceled,
            "task must be in Canceled state after cancel()"
        );
    }

    // ── tasks_for_agent ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn tasks_for_agent_returns_active_only() {
        let mut mgr = make_manager();
        let agent = AgentId::new();

        let task1 = mgr
            .create_task(make_message(), None, Some(agent), None, None)
            .expect("create task1 must succeed");
        let task2 = mgr
            .create_task(make_message(), None, Some(agent), None, None)
            .expect("create task2 must succeed");

        // Complete task1: Submitted → Working → Completed
        mgr.update_status(&task1.id, TaskState::Working, None)
            .expect("Submitted → Working");
        mgr.update_status(&task1.id, TaskState::Completed, None)
            .expect("Working → Completed");

        let active = mgr.tasks_for_agent(&agent);
        assert_eq!(
            active.len(),
            1,
            "only the non-terminal task must be returned"
        );
        assert_eq!(active[0].id, task2.id, "the active task must be task2");
    }

    // ── restore ───────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn restore_populates_tasks() {
        let mut mgr = make_manager();
        let original = mgr
            .create_task(make_message(), None, None, None, None)
            .expect("create must succeed");
        let snapshot = vec![original.clone()];

        let mut mgr2 = make_manager();
        mgr2.restore(snapshot);

        let found = mgr2
            .get(&original.id)
            .expect("restored task must be retrievable by id");
        assert_eq!(found.id, original.id, "restored task id must match");
        assert_eq!(
            found.context_id, original.context_id,
            "restored context_id must match"
        );
    }
}
