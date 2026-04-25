//! Persistent state management for the Trumpet daemon.
//!
//! [`StateManager`] uses an OpenDAL [`Operator`] to read and write
//! [`StateSnapshot`] blobs. The snapshot file is `snapshot.bin` within the
//! configured storage root and is encoded with `bincode` for compactness.

pub mod snapshot;

pub use snapshot::StateSnapshot;

use opendal::{Operator, services::Fs};

use crate::config::types::StorageConfig;
use crate::error::Error;

/// Manages durable snapshot I/O via an OpenDAL [`Operator`].
///
/// The manager owns only the operator and the path to the snapshot file;
/// all business logic for building and applying snapshots lives in
/// [`crate::server::state::AppState`].
pub struct StateManager {
    operator: Operator,
}

impl StateManager {
    /// Create a new [`StateManager`] backed by the filesystem at
    /// `config.path`.
    ///
    /// The directory is created if it does not already exist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StateSnapshotFailed`] when the OpenDAL operator cannot
    /// be initialised (e.g. the path is unwritable).
    pub async fn new(config: &StorageConfig) -> Result<Self, Error> {
        let root = config
            .path
            .to_str()
            .ok_or_else(|| Error::StateSnapshotFailed {
                reason: "storage path contains non-UTF-8 characters".to_owned(),
            })?;

        tokio::fs::create_dir_all(&config.path)
            .await
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!(
                    "creating storage directory '{}': {e}",
                    config.path.display()
                ),
            })?;

        let builder = Fs::default().root(root);
        let operator = Operator::new(builder)
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!("initialising storage operator: {e}"),
            })?
            .finish();

        Ok(Self { operator })
    }

    /// Serialise `snapshot` with bincode and persist it as `snapshot.bin`.
    ///
    /// An existing snapshot is atomically overwritten.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StateSnapshotFailed`] on encode or I/O failure.
    pub async fn save_snapshot(&self, snapshot: &StateSnapshot) -> Result<(), Error> {
        let bytes = serde_json::to_vec(snapshot).map_err(|e| Error::StateSnapshotFailed {
            reason: format!("encoding snapshot: {e}"),
        })?;

        self.operator
            .write("snapshot.bin", bytes)
            .await
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!("writing snapshot.bin: {e}"),
            })?;

        Ok(())
    }

    /// Load and deserialise the most recent snapshot, if one exists.
    ///
    /// Returns `Ok(None)` when no snapshot file is present.
    ///
    /// # Errors
    ///
    /// Returns [`Error::StateRestoreFailed`] on I/O or decode failure.
    pub async fn load_snapshot(&self) -> Result<Option<StateSnapshot>, Error> {
        match self.operator.read("snapshot.bin").await {
            Ok(buf) => {
                let bytes = buf.to_vec();
                let snapshot: StateSnapshot =
                    serde_json::from_slice(&bytes).map_err(|e| Error::StateRestoreFailed {
                        reason: format!("decoding snapshot.bin: {e}"),
                    })?;
                Ok(Some(snapshot))
            }
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::StateRestoreFailed {
                reason: format!("reading snapshot.bin: {e}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;
    use crate::config::types::StorageConfig;
    use crate::core::types::{
        AgentId, AgentInfo, AgentStatus, ChatMessage, Conversation, ConversationId, MessageId,
        SkillId, SkillInfo, SkillProvider,
    };

    async fn make_manager(dir: &TempDir) -> StateManager {
        let config = StorageConfig {
            backend: "fs".to_owned(),
            path: dir.path().to_path_buf(),
            snapshot_interval_secs: 60,
        };
        StateManager::new(&config)
            .await
            .expect("StateManager::new must succeed for a valid temp dir")
    }

    fn minimal_snapshot() -> StateSnapshot {
        let agent = AgentInfo {
            id: AgentId::new(),
            name: "test-agent".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        };
        StateSnapshot {
            agents: vec![agent],
            skills: vec![],
            conversations: vec![],
            messages: vec![],
        }
    }

    fn rich_snapshot() -> (StateSnapshot, AgentId, ConversationId) {
        let agent_id = AgentId::new();
        let conv_id = ConversationId::new();

        let agent = AgentInfo {
            id: agent_id,
            name: "rich-agent".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        };
        let skill = SkillInfo {
            id: SkillId::new(),
            name: "rich-skill".to_owned(),
            description: "a skill".to_owned(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            provider: SkillProvider::BuiltIn,
        };
        let conv = Conversation {
            id: conv_id,
            name: Some("planning".to_owned()),
            participants: HashSet::from([agent_id]),
            created_at: Utc::now(),
        };
        let msg = ChatMessage {
            id: MessageId::new(),
            conversation_id: conv_id,
            sender: agent_id,
            content: "hello".to_owned(),
            timestamp: Utc::now(),
        };

        let snapshot = StateSnapshot {
            agents: vec![agent],
            skills: vec![skill],
            conversations: vec![conv],
            messages: vec![(conv_id, vec![msg])],
        };

        (snapshot, agent_id, conv_id)
    }

    #[tokio::test]
    async fn save_and_load_snapshot() {
        let dir = TempDir::new().expect("temp dir must be created");
        let manager = make_manager(&dir).await;
        let (original, agent_id, conv_id) = rich_snapshot();

        manager
            .save_snapshot(&original)
            .await
            .expect("save_snapshot must succeed");

        let loaded = manager
            .load_snapshot()
            .await
            .expect("load_snapshot must succeed")
            .expect("snapshot must exist after save");

        assert_eq!(
            loaded.agents.len(),
            1,
            "loaded snapshot must contain 1 agent"
        );
        assert_eq!(
            loaded.agents[0].id, agent_id,
            "agent id must survive save/load"
        );
        assert_eq!(
            loaded.agents[0].name, "rich-agent",
            "agent name must survive save/load"
        );
        assert_eq!(
            loaded.skills.len(),
            1,
            "loaded snapshot must contain 1 skill"
        );
        assert_eq!(
            loaded.skills[0].name, "rich-skill",
            "skill name must survive save/load"
        );
        assert_eq!(
            loaded.conversations.len(),
            1,
            "loaded snapshot must contain 1 conversation"
        );
        assert_eq!(
            loaded.conversations[0].id, conv_id,
            "conversation id must survive save/load"
        );
        assert_eq!(
            loaded.messages.len(),
            1,
            "loaded snapshot must contain message history for 1 conversation"
        );
        let (loaded_conv_id, msgs) = &loaded.messages[0];
        assert_eq!(
            *loaded_conv_id, conv_id,
            "message key conversation id must survive save/load"
        );
        assert_eq!(msgs.len(), 1, "message must survive save/load");
        assert_eq!(
            msgs[0].content, "hello",
            "message content must survive save/load"
        );
    }

    #[tokio::test]
    async fn load_snapshot_returns_none_when_empty() {
        let dir = TempDir::new().expect("temp dir must be created");
        let manager = make_manager(&dir).await;

        let result = manager
            .load_snapshot()
            .await
            .expect("load_snapshot on empty store must not error");

        assert!(
            result.is_none(),
            "load_snapshot must return None when no snapshot exists"
        );
    }

    #[tokio::test]
    async fn save_overwrites_previous_snapshot() {
        let dir = TempDir::new().expect("temp dir must be created");
        let manager = make_manager(&dir).await;

        let first = minimal_snapshot();
        manager
            .save_snapshot(&first)
            .await
            .expect("first save must succeed");

        // Build a second snapshot with a different agent name.
        let second_agent = AgentInfo {
            id: AgentId::new(),
            name: "second-agent".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Disconnected,
        };
        let second = StateSnapshot {
            agents: vec![second_agent.clone()],
            skills: vec![],
            conversations: vec![],
            messages: vec![],
        };

        manager
            .save_snapshot(&second)
            .await
            .expect("second save must succeed");

        let loaded = manager
            .load_snapshot()
            .await
            .expect("load_snapshot must succeed")
            .expect("snapshot must exist after second save");

        assert_eq!(
            loaded.agents.len(),
            1,
            "loaded snapshot must contain exactly 1 agent (the second one)"
        );
        assert_eq!(
            loaded.agents[0].name, "second-agent",
            "loaded snapshot must reflect the second save, not the first"
        );
        assert_eq!(
            loaded.agents[0].id, second_agent.id,
            "second agent id must survive save/load"
        );
    }
}
