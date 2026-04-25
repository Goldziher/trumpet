//! Skill registry — tracks capabilities that agents or the system expose.
//!
//! Skills are registered with a name, description, and JSON schemas for
//! their input and output. Any component that wants to invoke a skill
//! first looks it up here by name or [`SkillId`].

use std::sync::Arc;

use ahash::AHashMap;

use crate::core::bus::{Event, MessageBus};
use crate::core::types::{SkillId, SkillInfo, SkillProvider};
use crate::error::Error;

/// Registry of all skills available in the nexus.
///
/// Skills are indexed by [`SkillId`] for O(1) lookup. A secondary
/// `name_index` provides O(1) name uniqueness checks and
/// [`Self::find_by_name`] lookups.
pub struct SkillRegistry {
    skills: AHashMap<SkillId, SkillInfo>,
    name_index: AHashMap<String, SkillId>,
    bus: Arc<MessageBus>,
}

/// Validates a skill name against the allowed character set.
///
/// Rules: 1–64 ASCII bytes, alphanumeric plus hyphens, underscores,
/// and dots. No leading/trailing hyphen or dot, no consecutive dots.
fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() {
        return Err(Error::SkillInvalidName {
            name: name.to_owned(),
            reason: "name must not be empty".to_owned(),
        });
    }

    if name.len() > 64 {
        return Err(Error::SkillInvalidName {
            name: name.to_owned(),
            reason: "name must be 64 characters or fewer".to_owned(),
        });
    }

    if name.starts_with('-') || name.ends_with('-') {
        return Err(Error::SkillInvalidName {
            name: name.to_owned(),
            reason: "name must not start or end with a hyphen".to_owned(),
        });
    }

    if name.starts_with('.') || name.ends_with('.') {
        return Err(Error::SkillInvalidName {
            name: name.to_owned(),
            reason: "name must not start or end with a dot".to_owned(),
        });
    }

    if name.contains("..") {
        return Err(Error::SkillInvalidName {
            name: name.to_owned(),
            reason: "name must not contain consecutive dots".to_owned(),
        });
    }

    let all_valid = name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.');

    if !all_valid {
        return Err(Error::SkillInvalidName {
            name: name.to_owned(),
            reason:
                "name may only contain ASCII alphanumeric characters, hyphens, underscores, and dots"
                    .to_owned(),
        });
    }

    Ok(())
}

impl SkillRegistry {
    /// Create a new, empty registry backed by the given [`MessageBus`].
    pub fn new(bus: Arc<MessageBus>) -> Self {
        Self {
            skills: AHashMap::new(),
            name_index: AHashMap::new(),
            bus,
        }
    }

    /// Register a new skill.
    ///
    /// Returns the newly created [`SkillInfo`] on success.
    ///
    /// # Errors
    ///
    /// - [`Error::SkillInvalidName`] — `name` fails validation rules.
    /// - [`Error::SkillAlreadyRegistered`] — a skill with `name` is already registered.
    pub fn register(
        &mut self,
        name: &str,
        description: &str,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
        provider: SkillProvider,
    ) -> Result<SkillInfo, Error> {
        validate_name(name)?;

        if description.is_empty() {
            return Err(Error::SkillInvalidDescription {
                name: name.to_owned(),
                reason: "description must not be empty".to_owned(),
            });
        }

        if self.name_index.contains_key(name) {
            return Err(Error::SkillAlreadyRegistered {
                name: name.to_owned(),
            });
        }

        let info = SkillInfo {
            id: SkillId::new(),
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema,
            output_schema,
            provider,
        };

        self.name_index.insert(info.name.clone(), info.id);
        self.skills.insert(info.id, info.clone());
        self.bus.publish(Event::SkillRegistered(info.clone()));

        Ok(info)
    }

    /// Remove the skill identified by `id` from the registry.
    ///
    /// Returns the removed [`SkillInfo`] on success.
    ///
    /// # Errors
    ///
    /// - [`Error::SkillNotFound`] — no skill with `id` is registered.
    pub fn deregister(&mut self, id: &SkillId) -> Result<SkillInfo, Error> {
        let info = self
            .skills
            .remove(id)
            .ok_or(Error::SkillNotFoundById { id: id.to_string() })?;

        self.name_index.remove(&info.name);
        self.bus.publish(Event::SkillDeregistered(*id));

        Ok(info)
    }

    /// Look up a skill by its [`SkillId`].
    pub fn get(&self, id: &SkillId) -> Option<&SkillInfo> {
        self.skills.get(id)
    }

    /// Return all registered skills in unspecified order.
    pub fn list(&self) -> Vec<&SkillInfo> {
        self.skills.values().collect()
    }

    /// Find a skill by human-readable name.
    ///
    /// Names are unique within the registry, so at most one result is returned.
    pub fn find_by_name(&self, name: &str) -> Option<&SkillInfo> {
        let id = self.name_index.get(name)?;
        self.skills.get(id)
    }

    /// Clear all entries and repopulate from `skills`.
    ///
    /// Used during daemon startup to restore persisted state. No bus events
    /// are published.
    pub fn restore(&mut self, skills: Vec<SkillInfo>) {
        self.skills.clear();
        self.name_index.clear();
        for info in skills {
            if validate_name(&info.name).is_err() {
                tracing::warn!(name = %info.name, "skipping skill with invalid name during restore");
                continue;
            }
            self.name_index.insert(info.name.clone(), info.id);
            self.skills.insert(info.id, info);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::bus::{Event, MessageBus};

    fn make_registry() -> SkillRegistry {
        let bus = Arc::new(MessageBus::new(16));
        SkillRegistry::new(bus)
    }

    fn null_schema() -> serde_json::Value {
        serde_json::json!({})
    }

    // ── register ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_skill_succeeds() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "scan_repo",
                "Scans a repository",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed for a valid, unique name");

        assert_eq!(
            info.name, "scan_repo",
            "returned SkillInfo must carry the registered name"
        );
        assert_eq!(
            info.description, "Scans a repository",
            "returned SkillInfo must carry the registered description"
        );
        assert_eq!(
            info.provider,
            SkillProvider::BuiltIn,
            "returned SkillInfo must carry the registered provider"
        );
    }

    #[tokio::test]
    async fn register_duplicate_name_fails() {
        let mut registry = make_registry();
        registry
            .register(
                "my_skill",
                "first",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("first registration must succeed");

        let err = registry
            .register(
                "my_skill",
                "second",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect_err("second registration with the same name must fail");

        assert!(
            matches!(err, Error::SkillAlreadyRegistered { ref name } if name == "my_skill"),
            "expected SkillAlreadyRegistered(\"my_skill\"), got: {err:?}"
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
                SkillProvider::BuiltIn,
            )
            .expect_err("empty name must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidName { ref name, .. } if name.is_empty()),
            "expected SkillInvalidName for empty string, got: {err:?}"
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
                SkillProvider::BuiltIn,
            )
            .expect_err("leading hyphen must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidName { .. }),
            "expected SkillInvalidName for leading hyphen, got: {err:?}"
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
                SkillProvider::BuiltIn,
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
                SkillProvider::BuiltIn,
            )
            .expect_err("leading dot must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidName { .. }),
            "expected SkillInvalidName for leading dot, got: {err:?}"
        );
    }

    // ── deregister ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn deregister_removes_skill() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "to-remove",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed");

        registry
            .deregister(&info.id)
            .expect("deregister must succeed for a known id");

        assert!(
            registry.list().is_empty(),
            "list must be empty after deregistering the only skill"
        );
    }

    #[tokio::test]
    async fn deregister_unknown_fails() {
        let mut registry = make_registry();
        let unknown_id = SkillId::new();

        let err = registry
            .deregister(&unknown_id)
            .expect_err("deregister with unknown id must fail");

        assert!(
            matches!(err, Error::SkillNotFoundById { .. }),
            "expected SkillNotFoundById, got: {err:?}"
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
                SkillProvider::BuiltIn,
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
                SkillProvider::BuiltIn,
            )
            .expect("re-registration with same name must succeed after deregister");

        assert_ne!(info.id, info2.id, "re-registered skill must have a new id");
    }

    // ── name validation edge cases ───────────────────────────────────────────

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
                SkillProvider::BuiltIn,
            )
            .expect_err("name exceeding 64 chars must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidName { .. }),
            "expected SkillInvalidName for overlong name, got: {err:?}"
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
                SkillProvider::BuiltIn,
            )
            .expect_err("trailing hyphen must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidName { .. }),
            "expected SkillInvalidName for trailing hyphen, got: {err:?}"
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
                SkillProvider::BuiltIn,
            )
            .expect_err("empty description must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidDescription { ref name, .. } if name == "valid-name"),
            "expected SkillInvalidDescription for empty description, got: {err:?}"
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
                SkillProvider::BuiltIn,
            )
            .expect_err("consecutive dots must be rejected");

        assert!(
            matches!(err, Error::SkillInvalidName { .. }),
            "expected SkillInvalidName for consecutive dots, got: {err:?}"
        );
    }

    // ── get / list / find_by_name ─────────────────────────────────────────────

    #[tokio::test]
    async fn get_returns_registered_skill() {
        let mut registry = make_registry();
        let info = registry
            .register(
                "lookup-me",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed");

        let found = registry
            .get(&info.id)
            .expect("get must return the registered skill");

        assert_eq!(
            found.id, info.id,
            "get must return the skill with the requested id"
        );
    }

    #[tokio::test]
    async fn list_returns_all_skills() {
        let mut registry = make_registry();
        registry
            .register(
                "alpha",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register alpha must succeed");
        registry
            .register(
                "beta",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register beta must succeed");

        let mut names: Vec<String> = registry.list().iter().map(|s| s.name.clone()).collect();
        names.sort_unstable();

        assert_eq!(
            names,
            vec!["alpha", "beta"],
            "list must return all registered skills"
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
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed");
        let beta = registry
            .register(
                "beta",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed");

        let found = registry
            .find_by_name("beta")
            .expect("find_by_name must locate a registered skill");

        assert_eq!(
            found.id, beta.id,
            "find_by_name must return the correct skill"
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

    // ── restore ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn restore_populates_skill_registry() {
        let mut registry = make_registry();
        let id1 = SkillId::new();
        let skills = vec![
            SkillInfo {
                id: id1,
                name: "skill-one".to_owned(),
                description: "first".to_owned(),
                input_schema: null_schema(),
                output_schema: null_schema(),
                provider: SkillProvider::BuiltIn,
            },
            SkillInfo {
                id: SkillId::new(),
                name: "skill-two".to_owned(),
                description: "second".to_owned(),
                input_schema: null_schema(),
                output_schema: null_schema(),
                provider: SkillProvider::BuiltIn,
            },
        ];

        registry.restore(skills);

        assert_eq!(registry.list().len(), 2, "restore must populate 2 skills");
        assert_eq!(
            registry.find_by_name("skill-one").unwrap().id,
            id1,
            "restored skill id must match"
        );
    }

    // ── bus events ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_publishes_skill_registered_event() {
        let bus = Arc::new(MessageBus::new(16));
        let mut registry = SkillRegistry::new(Arc::clone(&bus));
        let mut rx = bus.subscribe();

        let info = registry
            .register(
                "event-skill",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed");

        let event = rx
            .recv()
            .await
            .expect("bus must deliver the SkillRegistered event");

        let Event::SkillRegistered(received) = event else {
            panic!("expected SkillRegistered event, got something else");
        };

        assert_eq!(
            received.id, info.id,
            "event must carry the id of the registered skill"
        );
        assert_eq!(
            received.name, "event-skill",
            "event must carry the name of the registered skill"
        );
    }

    #[tokio::test]
    async fn deregister_publishes_skill_deregistered_event() {
        let bus = Arc::new(MessageBus::new(16));
        let mut registry = SkillRegistry::new(Arc::clone(&bus));

        let info = registry
            .register(
                "to-deregister",
                "desc",
                null_schema(),
                null_schema(),
                SkillProvider::BuiltIn,
            )
            .expect("register must succeed");

        let mut rx = bus.subscribe();

        registry
            .deregister(&info.id)
            .expect("deregister must succeed");

        let event = rx
            .recv()
            .await
            .expect("bus must deliver the SkillDeregistered event");

        let Event::SkillDeregistered(deregistered_id) = event else {
            panic!("expected SkillDeregistered event, got something else");
        };

        assert_eq!(
            deregistered_id, info.id,
            "event must carry the id of the deregistered skill"
        );
    }
}
