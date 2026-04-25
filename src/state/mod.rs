//! Persistent state management for the Trumpet daemon.
//!
//! [`StateManager`] uses an OpenDAL [`Operator`] to read and write
//! [`StateSnapshot`] blobs. Snapshots are serialized as MessagePack, then
//! encrypted with AES-256-GCM before writing to `snapshot.enc`.

pub mod crypto;
pub mod snapshot;

pub use snapshot::StateSnapshot;

use aes_gcm::Aes256Gcm;
use aes_gcm::Key;
use opendal::{Operator, services::Fs};

use crate::config::types::StorageConfig;
use crate::error::Error;

const SNAPSHOT_FILE: &str = "snapshot.enc";
const KEY_FILE: &str = "key";
const MAX_SNAPSHOT_SIZE: usize = 64 * 1024 * 1024; // 64 MiB

/// Manages durable snapshot I/O via an OpenDAL [`Operator`].
///
/// Snapshots are encrypted at rest with AES-256-GCM. The key is stored
/// alongside the snapshot (generated on first use).
pub struct StateManager {
    operator: Operator,
    key: Key<Aes256Gcm>,
}

impl StateManager {
    /// Create a new [`StateManager`] backed by the filesystem at
    /// `config.path`.
    ///
    /// The directory is created if it does not already exist. An encryption
    /// key is loaded from `{config.path}/key` or generated on first run.
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                tokio::fs::set_permissions(&config.path, std::fs::Permissions::from_mode(0o700))
                    .await;
        }

        let key = crypto::load_or_create_key(&config.path.join(KEY_FILE)).await?;

        let builder = Fs::default().root(root);
        let operator = Operator::new(builder)
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!("initialising storage operator: {e}"),
            })?
            .finish();

        Ok(Self { operator, key })
    }

    /// Serialize, encrypt, and persist `snapshot`.
    pub async fn save_snapshot(&self, snapshot: &StateSnapshot) -> Result<(), Error> {
        let plaintext = rmp_serde::to_vec(snapshot).map_err(|e| Error::StateSnapshotFailed {
            reason: format!("encoding snapshot: {e}"),
        })?;

        let encrypted = crypto::encrypt(&self.key, &plaintext)?;

        self.operator
            .write(SNAPSHOT_FILE, encrypted)
            .await
            .map_err(|e| Error::StateSnapshotFailed {
                reason: format!("writing {SNAPSHOT_FILE}: {e}"),
            })?;

        Ok(())
    }

    /// Load, decrypt, and deserialize the most recent snapshot.
    ///
    /// Returns `Ok(None)` when no snapshot file is present.
    pub async fn load_snapshot(&self) -> Result<Option<StateSnapshot>, Error> {
        match self.operator.read(SNAPSHOT_FILE).await {
            Ok(buf) => {
                let bytes = buf.to_vec();
                if bytes.len() > MAX_SNAPSHOT_SIZE {
                    return Err(Error::StateRestoreFailed {
                        reason: format!(
                            "snapshot too large ({} bytes, max {MAX_SNAPSHOT_SIZE})",
                            bytes.len()
                        ),
                    });
                }

                let plaintext = crypto::decrypt(&self.key, &bytes)?;

                let snapshot: StateSnapshot =
                    rmp_serde::from_slice(&plaintext).map_err(|e| Error::StateRestoreFailed {
                        reason: format!("decoding snapshot: {e}"),
                    })?;
                Ok(Some(snapshot))
            }
            Err(e) if e.kind() == opendal::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::StateRestoreFailed {
                reason: format!("reading {SNAPSHOT_FILE}: {e}"),
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
            tasks: vec![],
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
            tasks: vec![],
        };

        (snapshot, agent_id, conv_id)
    }

    #[tokio::test]
    async fn save_and_load_snapshot_with_encryption() {
        let dir = TempDir::new().unwrap();
        let manager = make_manager(&dir).await;
        let (original, agent_id, conv_id) = rich_snapshot();

        manager.save_snapshot(&original).await.unwrap();

        // Verify the file on disk is not plaintext msgpack.
        let raw = tokio::fs::read(dir.path().join(SNAPSHOT_FILE))
            .await
            .unwrap();
        assert!(
            rmp_serde::from_slice::<StateSnapshot>(&raw).is_err(),
            "raw file must not be valid msgpack (it should be encrypted)"
        );

        let loaded = manager.load_snapshot().await.unwrap().unwrap();

        assert_eq!(loaded.agents[0].id, agent_id);
        assert_eq!(loaded.agents[0].name, "rich-agent");
        assert_eq!(loaded.skills[0].name, "rich-skill");
        assert_eq!(loaded.conversations[0].id, conv_id);
        assert_eq!(loaded.messages[0].1[0].content, "hello");
    }

    #[tokio::test]
    async fn load_snapshot_returns_none_when_empty() {
        let dir = TempDir::new().unwrap();
        let manager = make_manager(&dir).await;
        assert!(manager.load_snapshot().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn save_overwrites_previous_snapshot() {
        let dir = TempDir::new().unwrap();
        let manager = make_manager(&dir).await;

        manager.save_snapshot(&minimal_snapshot()).await.unwrap();

        let second = StateSnapshot {
            agents: vec![AgentInfo {
                id: AgentId::new(),
                name: "second".to_owned(),
                registered_at: Utc::now(),
                status: AgentStatus::Disconnected,
            }],
            skills: vec![],
            conversations: vec![],
            messages: vec![],
            tasks: vec![],
        };
        manager.save_snapshot(&second).await.unwrap();

        let loaded = manager.load_snapshot().await.unwrap().unwrap();
        assert_eq!(loaded.agents[0].name, "second");
    }

    #[tokio::test]
    async fn different_key_cannot_decrypt() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        let mgr1 = make_manager(&dir1).await;

        mgr1.save_snapshot(&minimal_snapshot()).await.unwrap();

        // Copy the encrypted file to dir2 but with a different key.
        let encrypted = tokio::fs::read(dir1.path().join(SNAPSHOT_FILE))
            .await
            .unwrap();
        tokio::fs::write(dir2.path().join(SNAPSHOT_FILE), &encrypted)
            .await
            .unwrap();

        let mgr2 = make_manager(&dir2).await;
        let result = mgr2.load_snapshot().await;
        assert!(result.is_err(), "decryption with wrong key must fail");
    }
}
