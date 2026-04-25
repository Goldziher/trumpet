//! Agent registry — tracks registered agents by [`AgentId`] and name.
//!
//! The registry is intentionally not wrapped in `Arc`/`RwLock`; that
//! belongs at the server layer where locking strategy is decided.

use std::sync::Arc;

use ahash::AHashMap;
use chrono::Utc;

use crate::core::bus::{Event, MessageBus};
use crate::core::types::{AgentId, AgentInfo, AgentStatus};
use crate::error::Error;

/// Validates an agent name against the allowed character set.
///
/// Rules: 1–64 characters, ASCII alphanumeric plus hyphens and underscores,
/// no leading or trailing hyphen.
fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::AgentInvalidName {
            name: name.to_owned(),
            reason: "name must not be empty".to_owned(),
        });
    }

    if name.len() > 64 {
        return Err(Error::AgentInvalidName {
            name: name.to_owned(),
            reason: "name must be 64 characters or fewer".to_owned(),
        });
    }

    if name.starts_with('-') || name.ends_with('-') {
        return Err(Error::AgentInvalidName {
            name: name.to_owned(),
            reason: "name must not start or end with a hyphen".to_owned(),
        });
    }

    let all_valid = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');

    if !all_valid {
        return Err(Error::AgentInvalidName {
            name: name.to_owned(),
            reason: "name may only contain ASCII alphanumeric characters, hyphens, and underscores"
                .to_owned(),
        });
    }

    Ok(())
}

/// Registry of all agents currently connected to the nexus.
///
/// Agents are indexed by [`AgentId`] for O(1) lookup. A secondary
/// `name_index` provides O(1) name uniqueness checks and
/// [`Self::find_by_name`] lookups.
pub struct AgentRegistry {
    agents: AHashMap<AgentId, AgentInfo>,
    name_index: AHashMap<String, AgentId>,
    bus: Arc<MessageBus>,
}

impl AgentRegistry {
    /// Create a new, empty registry backed by the given [`MessageBus`].
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            agents: AHashMap::new(),
            name_index: AHashMap::new(),
            bus,
        }
    }

    /// Register a new agent with the given `name`.
    ///
    /// Returns the newly created [`AgentInfo`] on success.
    ///
    /// # Errors
    ///
    /// - [`Error::AgentInvalidName`] — `name` fails validation rules.
    /// - [`Error::AgentAlreadyRegistered`] — an agent with `name` is already registered.
    pub fn register(&mut self, name: &str) -> Result<AgentInfo, Error> {
        validate_name(name)?;

        if self.name_index.contains_key(name) {
            return Err(Error::AgentAlreadyRegistered {
                name: name.to_owned(),
            });
        }

        let info = AgentInfo {
            id: AgentId::new(),
            name: name.to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        };

        self.name_index.insert(info.name.clone(), info.id);
        self.agents.insert(info.id, info.clone());
        self.bus.publish(Event::AgentRegistered(info.clone()));

        Ok(info)
    }

    /// Remove the agent identified by `id` from the registry.
    ///
    /// Returns the removed [`AgentInfo`] on success.
    ///
    /// # Errors
    ///
    /// - [`Error::AgentNotFound`] — no agent with `id` is registered.
    pub fn deregister(&mut self, id: &AgentId) -> Result<AgentInfo, Error> {
        let info = self.agents.remove(id).ok_or_else(|| Error::AgentNotFound {
            name: id.to_string(),
        })?;

        self.name_index.remove(&info.name);
        self.bus.publish(Event::AgentDeregistered(*id));

        Ok(info)
    }

    /// Look up an agent by its [`AgentId`].
    pub fn get(&self, id: &AgentId) -> Option<&AgentInfo> {
        self.agents.get(id)
    }

    /// Return all registered agents in unspecified order.
    pub fn list(&self) -> Vec<&AgentInfo> {
        self.agents.values().collect()
    }

    /// Find an agent by human-readable name.
    ///
    /// Names are unique within the registry, so at most one result is returned.
    pub fn find_by_name(&self, name: &str) -> Option<&AgentInfo> {
        let id = self.name_index.get(name)?;
        self.agents.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> AgentRegistry {
        let bus = Arc::new(MessageBus::new(16));
        AgentRegistry::new(bus)
    }

    // ── register ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_agent_succeeds() {
        let mut registry = make_registry();
        let info = registry
            .register("claude-code-1")
            .expect("register must succeed for a valid, unique name");

        assert_eq!(
            info.name, "claude-code-1",
            "returned AgentInfo must carry the registered name"
        );
        assert_eq!(
            info.status,
            AgentStatus::Connected,
            "newly registered agent must start as Connected"
        );
    }

    #[tokio::test]
    async fn register_duplicate_name_fails() {
        let mut registry = make_registry();
        registry
            .register("worker")
            .expect("first registration must succeed");

        let err = registry
            .register("worker")
            .expect_err("second registration with the same name must fail");

        assert!(
            matches!(err, Error::AgentAlreadyRegistered { ref name } if name == "worker"),
            "expected AgentAlreadyRegistered(\"worker\"), got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_invalid_name_fails_empty_string() {
        let mut registry = make_registry();
        let err = registry
            .register("")
            .expect_err("empty name must be rejected");

        assert!(
            matches!(err, Error::AgentInvalidName { ref name, .. } if name.is_empty()),
            "expected AgentInvalidName for empty string, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_too_long_fails() {
        let mut registry = make_registry();
        let long_name = "a".repeat(65);
        let err = registry
            .register(&long_name)
            .expect_err("name exceeding 64 chars must be rejected");

        assert!(
            matches!(err, Error::AgentInvalidName { .. }),
            "expected AgentInvalidName for overlong name, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_with_leading_hyphen_fails() {
        let mut registry = make_registry();
        let err = registry
            .register("-bad")
            .expect_err("leading hyphen must be rejected");

        assert!(
            matches!(err, Error::AgentInvalidName { .. }),
            "expected AgentInvalidName for leading hyphen, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_with_trailing_hyphen_fails() {
        let mut registry = make_registry();
        let err = registry
            .register("bad-")
            .expect_err("trailing hyphen must be rejected");

        assert!(
            matches!(err, Error::AgentInvalidName { .. }),
            "expected AgentInvalidName for trailing hyphen, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn register_name_with_invalid_char_fails() {
        let mut registry = make_registry();
        let err = registry
            .register("bad name")
            .expect_err("space in name must be rejected");

        assert!(
            matches!(err, Error::AgentInvalidName { .. }),
            "expected AgentInvalidName for space in name, got: {err:?}"
        );
    }

    // ── deregister ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn deregister_removes_agent() {
        let mut registry = make_registry();
        let info = registry
            .register("to-remove")
            .expect("register must succeed");

        registry
            .deregister(&info.id)
            .expect("deregister must succeed for a known id");

        assert!(
            registry.list().is_empty(),
            "list must be empty after deregistering the only agent"
        );
    }

    #[tokio::test]
    async fn deregister_unknown_fails() {
        let mut registry = make_registry();
        let unknown_id = AgentId::new();

        let err = registry
            .deregister(&unknown_id)
            .expect_err("deregister with unknown id must fail");

        assert!(
            matches!(err, Error::AgentNotFound { .. }),
            "expected AgentNotFound, got: {err:?}"
        );
    }

    // ── get / list / find_by_name ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_returns_registered_agent() {
        let mut registry = make_registry();
        let info = registry
            .register("lookup-me")
            .expect("register must succeed");

        let found = registry
            .get(&info.id)
            .expect("get must return the registered agent");

        assert_eq!(
            found.id, info.id,
            "get must return the agent with the requested id"
        );
    }

    #[tokio::test]
    async fn find_by_name_works() {
        let mut registry = make_registry();
        registry.register("alpha").expect("register must succeed");
        let beta = registry.register("beta").expect("register must succeed");

        let found = registry
            .find_by_name("beta")
            .expect("find_by_name must locate a registered agent");

        assert_eq!(
            found.id, beta.id,
            "find_by_name must return the correct agent"
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

    // ── bus events ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_publishes_agent_registered_event() {
        let bus = Arc::new(MessageBus::new(16));
        let mut registry = AgentRegistry::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        let info = registry
            .register("event-agent")
            .expect("register must succeed");

        let event = rx
            .recv()
            .await
            .expect("bus must deliver the AgentRegistered event");

        let Event::AgentRegistered(received) = event else {
            panic!("expected AgentRegistered event, got something else");
        };

        assert_eq!(
            received.id, info.id,
            "event must carry the id of the registered agent"
        );
        assert_eq!(
            received.name, "event-agent",
            "event must carry the name of the registered agent"
        );
    }

    #[tokio::test]
    async fn deregister_publishes_agent_deregistered_event() {
        let bus = Arc::new(MessageBus::new(16));
        let mut registry = AgentRegistry::new(Arc::clone(&bus));

        let info = registry
            .register("to-deregister")
            .expect("register must succeed");

        let mut rx = bus.subscribe();

        registry
            .deregister(&info.id)
            .expect("deregister must succeed");

        let event = rx
            .recv()
            .await
            .expect("bus must deliver the AgentDeregistered event");

        let Event::AgentDeregistered(deregistered_id) = event else {
            panic!("expected AgentDeregistered event, got something else");
        };

        assert_eq!(
            deregistered_id, info.id,
            "event must carry the id of the deregistered agent"
        );
    }
}
