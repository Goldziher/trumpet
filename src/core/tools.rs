//! Tool registry — tracks capabilities that agents or the system expose.
//!
//! Tools are registered with a name, description, and JSON schemas for
//! their input and output. Any component that wants to invoke a tool
//! first looks it up here by name or [`ToolId`].

use std::sync::Arc;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use crate::core::bus::{Event, MessageBus};
use crate::core::task_types::Task;
use crate::core::types::{ToolId, ToolInfo, ToolProvider};
use crate::error::Error;

/// Result of invoking a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResult {
    /// Immediate result from a built-in tool.
    Immediate { output: serde_json::Value },
    /// Asynchronous result — a task was created and assigned to an agent.
    TaskCreated { task: Box<Task> },
}

/// Registry of all tools available in the nexus.
///
/// Tools are indexed by [`ToolId`] for O(1) lookup. A secondary
/// `name_index` provides O(1) name uniqueness checks and
/// [`Self::find_by_name`] lookups.
pub struct ToolRegistry {
    tools: AHashMap<ToolId, ToolInfo>,
    name_index: AHashMap<String, ToolId>,
    bus: Arc<MessageBus>,
}

/// Validates a tool name against the allowed character set.
///
/// Rules: 1–64 ASCII bytes, alphanumeric plus hyphens, underscores,
/// and dots. No leading/trailing hyphen or dot, no consecutive dots.
fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::ToolInvalidName {
            name: name.to_owned(),
            reason: "name must not be empty".to_owned(),
        });
    }

    if name.len() > 64 {
        return Err(Error::ToolInvalidName {
            name: name.to_owned(),
            reason: "name must be 64 characters or fewer".to_owned(),
        });
    }

    if name.starts_with('-') || name.ends_with('-') {
        return Err(Error::ToolInvalidName {
            name: name.to_owned(),
            reason: "name must not start or end with a hyphen".to_owned(),
        });
    }

    if name.starts_with('.') || name.ends_with('.') {
        return Err(Error::ToolInvalidName {
            name: name.to_owned(),
            reason: "name must not start or end with a dot".to_owned(),
        });
    }

    if name.contains("..") {
        return Err(Error::ToolInvalidName {
            name: name.to_owned(),
            reason: "name must not contain consecutive dots".to_owned(),
        });
    }

    let all_valid = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');

    if !all_valid {
        return Err(Error::ToolInvalidName {
            name: name.to_owned(),
            reason:
                "name may only contain ASCII alphanumeric characters, hyphens, underscores, and dots"
                    .to_owned(),
        });
    }

    Ok(())
}

impl ToolRegistry {
    /// Create a new, empty registry backed by the given [`MessageBus`].
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            tools: AHashMap::new(),
            name_index: AHashMap::new(),
            bus,
        }
    }

    /// Register a new tool.
    ///
    /// Returns the newly created [`ToolInfo`] on success.
    ///
    /// # Errors
    ///
    /// - [`Error::ToolInvalidName`] — `name` fails validation rules.
    /// - [`Error::ToolAlreadyRegistered`] — a tool with `name` is already registered.
    pub fn register(
        &mut self,
        name: &str,
        description: &str,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
        provider: ToolProvider,
    ) -> Result<ToolInfo, Error> {
        validate_name(name)?;

        if description.is_empty() {
            return Err(Error::ToolInvalidDescription {
                name: name.to_owned(),
                reason: "description must not be empty".to_owned(),
            });
        }

        if self.name_index.contains_key(name) {
            return Err(Error::ToolAlreadyRegistered {
                name: name.to_owned(),
            });
        }

        let info = ToolInfo {
            id: ToolId::new(),
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema,
            output_schema,
            provider,
        };

        self.name_index.insert(info.name.clone(), info.id);
        self.tools.insert(info.id, info.clone());
        self.bus.publish(Event::ToolRegistered(info.clone()));

        Ok(info)
    }

    /// Remove the tool identified by `id` from the registry.
    ///
    /// Returns the removed [`ToolInfo`] on success.
    ///
    /// # Errors
    ///
    /// - [`Error::ToolNotFoundById`] — no tool with `id` is registered.
    pub fn deregister(&mut self, id: &ToolId) -> Result<ToolInfo, Error> {
        let info = self
            .tools
            .remove(id)
            .ok_or(Error::ToolNotFoundById { id: id.to_string() })?;

        self.name_index.remove(&info.name);
        self.bus.publish(Event::ToolDeregistered(*id));

        Ok(info)
    }

    /// Look up a tool by its [`ToolId`].
    pub fn get(&self, id: &ToolId) -> Option<&ToolInfo> {
        self.tools.get(id)
    }

    /// Return all registered tools in unspecified order.
    pub fn list(&self) -> Vec<&ToolInfo> {
        self.tools.values().collect()
    }

    /// Find a tool by human-readable name.
    ///
    /// Names are unique within the registry, so at most one result is returned.
    pub fn find_by_name(&self, name: &str) -> Option<&ToolInfo> {
        let id = self.name_index.get(name)?;
        self.tools.get(id)
    }

    /// Clear all entries and repopulate from `tools`.
    ///
    /// Used during daemon startup to restore persisted state. No bus events
    /// are published.
    pub fn restore(&mut self, tools: Vec<ToolInfo>) {
        self.tools.clear();
        self.name_index.clear();
        for info in tools {
            if validate_name(&info.name).is_err() {
                tracing::warn!(name = %info.name, "skipping tool with invalid name during restore");
                continue;
            }
            self.name_index.insert(info.name.clone(), info.id);
            self.tools.insert(info.id, info);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::bus::{Event, MessageBus};

    fn make_registry() -> ToolRegistry {
        let bus = Arc::new(MessageBus::new(16));
        ToolRegistry::new(bus)
    }

    fn null_schema() -> serde_json::Value {
        serde_json::json!({})
    }

    // ── register ──────────────────────────────────────────────────────���───────

    #[tokio::test]
    async fn register_tool_succeeds() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "scan_repo",
                "Scans a repository",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed for a valid, unique name");

        assert_eq!(
            info.name, "scan_repo",
            "returned ToolInfo must carry the registered name"
        );
        assert_eq!(
            info.description, "Scans a repository",
            "returned ToolInfo must carry the registered description"
        );
        assert_eq!(
            info.provider,
            ToolProvider::BuiltIn,
            "returned ToolInfo must carry the registered provider"
        );
    }

    #[tokio::test]
    async fn register_duplicate_name_fails() {
        let mut registry = make_registry();
        registry
            .register(
                "my_tool",
                "first",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("first registration must succeed");

        let err = registry
            .register(
                "my_tool",
                "second",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("second registration with the same name must fail");

        assert!(
            matches!(err, Error::ToolAlreadyRegistered { ref name } if name == "my_tool"),
            "expected ToolAlreadyRegistered(\"my_tool\"), got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_invalid_name_empty_fails() {
        let mut registry = make_registry();
        let err = registry
            .register(
                "",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("empty name must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidName { ref name, .. } if name.is_empty()),
            "expected ToolInvalidName for empty string, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_invalid_name_leading_hyphen_fails() {
        let mut registry = make_registry();
        let err = registry
            .register(
                "-bad",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("leading hyphen must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidName { .. }),
            "expected ToolInvalidName for leading hyphen, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_with_dots_succeeds() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "code.scan_repo",
                "Scans a repo",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("dotted name must be accepted");

        assert_eq!(
            info.name, "code.scan_repo",
            "dotted name must be preserved exactly"
        );
    }

    #[tokio::test]
    async fn register_name_with_leading_dot_fails() {
        let mut registry = make_registry();
        let err = registry
            .register(
                ".bad",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("leading dot must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidName { .. }),
            "expected ToolInvalidName for leading dot, got: {err:?}"
        );
    }

    // ── deregister ─────────────���──────────────────────────────────────────────

    #[tokio::test]
    async fn deregister_removes_tool() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "to-remove",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed");

        registry
            .deregister(&info.id)
            .expect("deregister must succeed for a known id");

        assert!(
            registry.list().is_empty(),
            "list must be empty after deregistering the only tool"
        );
    }

    #[tokio::test]
    async fn deregister_unknown_fails() {
        let mut registry = make_registry();
        let unknown_id = ToolId::new();

        let err = registry
            .deregister(&unknown_id)
            .expect_err("deregister with unknown id must fail");

        assert!(
            matches!(err, Error::ToolNotFoundById { .. }),
            "expected ToolNotFoundById, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn deregister_frees_name_slot_for_reregistration() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "reusable",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("first register must succeed");

        registry
            .deregister(&info.id)
            .expect("deregister must succeed");

        let info2 = registry
            .register(
                "reusable",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("re-registration with same name must succeed after deregister");

        assert_ne!(info.id, info2.id, "re-registered tool must have a new id");
    }

    // ── name validation edge cases ───��───────────────────────────���───────────

    #[tokio::test]
    async fn register_name_too_long_fails() {
        let mut registry = make_registry();
        let long_name = "a".repeat(65);
        let err = registry
            .register(
                &long_name,
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("name exceeding 64 chars must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidName { .. }),
            "expected ToolInvalidName for overlong name, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_with_trailing_hyphen_fails() {
        let mut registry = make_registry();
        let err = registry
            .register(
                "bad-",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("trailing hyphen must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidName { .. }),
            "expected ToolInvalidName for trailing hyphen, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_empty_description_fails() {
        let mut registry = make_registry();
        let err = registry
            .register(
                "valid-name",
                "",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("empty description must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidDescription { ref name, .. } if name == "valid-name"),
            "expected ToolInvalidDescription for empty description, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_with_consecutive_dots_fails() {
        let mut registry = make_registry();
        let err = registry
            .register(
                "code..scan",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect_err("consecutive dots must be rejected");

        assert!(
            matches!(err, Error::ToolInvalidName { .. }),
            "expected ToolInvalidName for consecutive dots, got: {err:?}"
        );
    }

    // ── get / list / find_by_name ────────────────────────────���────────────────

    #[tokio::test]
    async fn get_returns_registered_tool() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "lookup-me",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed");

        let found = registry
            .get(&info.id)
            .expect("get must return the registered tool");

        assert_eq!(
            found.id, info.id,
            "get must return the tool with the requested id"
        );
    }

    #[tokio::test]
    async fn list_returns_all_tools() {
        let mut registry = make_registry();
        registry
            .register(
                "alpha",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register alpha must succeed");
        registry
            .register(
                "beta",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register beta must succeed");

        let mut names: Vec<String> = registry.list().iter().map(|s| s.name.clone()).collect();
        names.sort_unstable();

        assert_eq!(
            names,
            vec!["alpha", "beta"],
            "list must return all registered tools"
        );
    }

    #[tokio::test]
    async fn find_by_name_works() {
        let mut registry = make_registry();
        registry
            .register(
                "alpha",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed");
        let beta = registry
            .register(
                "beta",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed");

        let found = registry
            .find_by_name("beta")
            .expect("find_by_name must locate a registered tool");

        assert_eq!(
            found.id, beta.id,
            "find_by_name must return the correct tool"
        );
    }

    #[tokio::test]
    async fn find_by_name_returns_none_for_unknown() {
        let registry = make_registry();
        assert!(
            registry.find_by_name("ghost").is_none(),
            "find_by_name must return None when name is not registered"
        );
    }

    // ── restore ──────────────────���───────────────────────────────────────────

    #[tokio::test]
    async fn restore_populates_tool_registry() {
        let mut registry = make_registry();
        let id1 = ToolId::new();
        let tools = vec![
            ToolInfo {
                id: id1,
                name: "tool-one".to_owned(),
                description: "first".to_owned(),
                input_schema: null_schema(),
                output_schema: null_schema(),
                provider: ToolProvider::BuiltIn,
            },
            ToolInfo {
                id: ToolId::new(),
                name: "tool-two".to_owned(),
                description: "second".to_owned(),
                input_schema: null_schema(),
                output_schema: null_schema(),
                provider: ToolProvider::BuiltIn,
            },
        ];

        registry.restore(tools);

        assert_eq!(registry.list().len(), 2, "restore must populate 2 tools");
        assert_eq!(
            registry.find_by_name("tool-one").unwrap().id,
            id1,
            "restored tool id must match"
        );
    }

    // ── bus events ────────────────��─────────────────────────────���─────────────

    #[tokio::test]
    async fn register_publishes_tool_registered_event() {
        let bus = Arc::new(MessageBus::new(16));
        let mut registry = ToolRegistry::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        let info = registry
            .register(
                "event-tool",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed");

        let event = rx
            .recv()
            .await
            .expect("bus must deliver the ToolRegistered event");

        let Event::ToolRegistered(received) = event else {
            panic!("expected ToolRegistered event, got something else");
        };

        assert_eq!(
            received.id, info.id,
            "event must carry the id of the registered tool"
        );
        assert_eq!(
            received.name, "event-tool",
            "event must carry the name of the registered tool"
        );
    }

    #[tokio::test]
    async fn deregister_publishes_tool_deregistered_event() {
        let bus = Arc::new(MessageBus::new(16));
        let mut registry = ToolRegistry::new(Arc::clone(&bus));

        let info = registry
            .register(
                "to-deregister",
                "desc",
                null_schema(),
                null_schema(),
                ToolProvider::BuiltIn,
            )
            .expect("register must succeed");

        let mut rx = bus.subscribe();

        registry
            .deregister(&info.id)
            .expect("deregister must succeed");

        let event = rx
            .recv()
            .await
            .expect("bus must deliver the ToolDeregistered event");

        let Event::ToolDeregistered(deregistered_id) = event else {
            panic!("expected ToolDeregistered event, got something else");
        };

        assert_eq!(
            deregistered_id, info.id,
            "event must carry the id of the deregistered tool"
        );
    }
}
