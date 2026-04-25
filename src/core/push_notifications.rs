//! Webhook push-notification configuration and delivery for tasks.
//!
//! Implements the four `*PushNotificationConfig` RPCs from the A2A spec
//! plus a background worker that subscribes to the message bus and POSTs
//! task lifecycle events to each registered webhook URL.
//!
//! Configurations are kept in [`PushNotificationStore`], indexed by
//! [`TaskId`]. The store is intentionally not wrapped in `Arc`/`RwLock`;
//! locking belongs at the server layer (it lives behind an
//! `Arc<RwLock<_>>` inside [`AppState`](crate::server::AppState)).

use std::sync::Arc;
use std::time::Duration;

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::core::bus::{Event, MessageBus};
use crate::core::task_types::TaskId;
use crate::error::Error;

// ── ID newtype ────────────────────────────────────────────────────────────────

/// Identifier for a single push-notification configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PushNotificationId(Uuid);

impl PushNotificationId {
    /// Mint a new random identifier.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PushNotificationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for PushNotificationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::str::FromStr for PushNotificationId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self(s.parse()?))
    }
}

// ── Authentication ────────────────────────────────────────────────────────────

/// HTTP authentication credentials used when delivering a webhook.
///
/// Currently the worker recognises `Bearer` and `Basic` schemes; other
/// schemes are still stored but the worker emits the credentials verbatim
/// in an `Authorization` header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushNotificationAuth {
    /// HTTP authentication scheme name (case-insensitive per RFC 9110).
    pub scheme: String,
    /// Credential payload — format depends on the scheme.
    pub credentials: String,
}

// ── Config ────────────────────────────────────────────────────────────────────

/// A single webhook configuration attached to a task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushNotificationConfig {
    /// Unique identifier for this configuration.
    pub id: PushNotificationId,
    /// Task this configuration is bound to.
    pub task_id: TaskId,
    /// Absolute URL to which lifecycle events are POSTed.
    pub url: String,
    /// Opaque token forwarded as `X-Trumpet-Notification-Token` so the
    /// receiver can correlate calls to a specific subscription.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
    /// Optional `Authorization` header credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authentication: Option<PushNotificationAuth>,
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// In-memory store of [`PushNotificationConfig`]s, indexed by task.
#[derive(Debug, Default)]
pub struct PushNotificationStore {
    configs: AHashMap<TaskId, Vec<PushNotificationConfig>>,
}

impl PushNotificationStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new webhook for `task_id` and return the populated
    /// configuration.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] when `url` is not a parseable absolute
    /// URL with an `http` or `https` scheme.
    pub fn create(
        &mut self,
        task_id: TaskId,
        url: String,
        token: String,
        authentication: Option<PushNotificationAuth>,
    ) -> Result<PushNotificationConfig, Error> {
        let parsed: url::Url = url.parse().map_err(|e| Error::InvalidInput {
            reason: format!("push notification url '{url}' is invalid: {e}"),
        })?;
        let scheme = parsed.scheme();
        if !matches!(scheme, "http" | "https") {
            return Err(Error::InvalidInput {
                reason: format!(
                    "push notification url '{url}' must use http or https; got '{scheme}'"
                ),
            });
        }

        let cfg = PushNotificationConfig {
            id: PushNotificationId::new(),
            task_id,
            url,
            token,
            authentication,
        };
        self.configs.entry(task_id).or_default().push(cfg.clone());
        Ok(cfg)
    }

    /// Fetch a single configuration by `(task_id, id)`.
    pub fn get(
        &self,
        task_id: &TaskId,
        id: &PushNotificationId,
    ) -> Option<&PushNotificationConfig> {
        self.configs.get(task_id)?.iter().find(|c| &c.id == id)
    }

    /// Return every configuration registered against `task_id`.
    pub fn list(&self, task_id: &TaskId) -> Vec<PushNotificationConfig> {
        self.configs.get(task_id).cloned().unwrap_or_default()
    }

    /// Delete the configuration `(task_id, id)`. Returns `true` when a
    /// configuration was removed.
    pub fn delete(&mut self, task_id: &TaskId, id: &PushNotificationId) -> bool {
        let Some(v) = self.configs.get_mut(task_id) else {
            return false;
        };
        let len_before = v.len();
        v.retain(|c| &c.id != id);
        let removed = v.len() < len_before;
        if v.is_empty() {
            self.configs.remove(task_id);
        }
        removed
    }

    /// Replace the entire store with `configs` (used by snapshot restore).
    pub fn restore(&mut self, configs: Vec<PushNotificationConfig>) {
        self.configs.clear();
        for cfg in configs {
            self.configs.entry(cfg.task_id).or_default().push(cfg);
        }
    }

    /// Flatten every stored configuration for snapshot persistence.
    pub fn all(&self) -> Vec<PushNotificationConfig> {
        self.configs.values().flatten().cloned().collect()
    }
}

// ── Webhook delivery ──────────────────────────────────────────────────────────

/// Default total timeout budget per webhook delivery attempt.
const DELIVERY_TIMEOUT_SECS: u64 = 10;

/// Maximum retry attempts after the initial delivery fails.
const MAX_RETRIES: u32 = 3;

/// Spawn a long-running task that subscribes to the message bus and posts
/// task lifecycle events to every registered webhook.
///
/// The returned [`tokio::task::JoinHandle`] should be aborted on shutdown.
/// Failures (network, non-2xx response) are logged at WARN level after the
/// retry budget is exhausted; they never panic the worker.
pub fn spawn_delivery_worker(
    store: Arc<RwLock<PushNotificationStore>>,
    bus: Arc<MessageBus>,
    client: reqwest::Client,
) -> tokio::task::JoinHandle<()> {
    let mut bus_rx = bus.subscribe();
    tokio::spawn(async move {
        loop {
            let event = match bus_rx.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        skipped = n,
                        "push notification worker lagged; events were dropped"
                    );
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };

            let Some(task_id) = task_id_for_event(&event) else {
                continue;
            };

            let configs = {
                let store = store.read().await;
                store.list(&task_id)
            };
            if configs.is_empty() {
                continue;
            }

            for cfg in configs {
                let payload = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
                let client = client.clone();
                tokio::spawn(deliver_with_retries(client, cfg, payload));
            }
        }
    })
}

/// Inspect a bus [`Event`] and return the task it pertains to, if any.
fn task_id_for_event(event: &Event) -> Option<TaskId> {
    match event {
        Event::TaskCreated(task) => Some(task.id),
        Event::TaskStatusChanged { task_id, .. } => Some(*task_id),
        Event::TaskArtifactAdded { task_id, .. } => Some(*task_id),
        _ => None,
    }
}

/// Attempt webhook delivery with exponential backoff between retries.
async fn deliver_with_retries(
    client: reqwest::Client,
    cfg: PushNotificationConfig,
    payload: serde_json::Value,
) {
    let mut attempt = 0u32;
    let mut delay = Duration::from_secs(1);
    loop {
        match deliver_once(&client, &cfg, &payload).await {
            Ok(()) => {
                tracing::debug!(
                    cfg_id = %cfg.id,
                    task_id = %cfg.task_id,
                    "push notification delivered"
                );
                return;
            }
            Err(reason) if attempt < MAX_RETRIES => {
                tracing::warn!(
                    cfg_id = %cfg.id,
                    task_id = %cfg.task_id,
                    attempt = attempt + 1,
                    reason,
                    "push notification delivery failed; retrying"
                );
                attempt += 1;
                tokio::time::sleep(delay).await;
                delay = delay.saturating_mul(2);
            }
            Err(reason) => {
                tracing::warn!(
                    cfg_id = %cfg.id,
                    task_id = %cfg.task_id,
                    reason,
                    "push notification delivery permanently failed"
                );
                return;
            }
        }
    }
}

async fn deliver_once(
    client: &reqwest::Client,
    cfg: &PushNotificationConfig,
    payload: &serde_json::Value,
) -> Result<(), String> {
    let mut request = client
        .post(&cfg.url)
        .timeout(Duration::from_secs(DELIVERY_TIMEOUT_SECS))
        .json(payload);

    if !cfg.token.is_empty() {
        request = request.header("X-Trumpet-Notification-Token", &cfg.token);
    }
    if let Some(auth) = &cfg.authentication {
        let header_value = format!("{} {}", auth.scheme, auth.credentials);
        request = request.header("Authorization", header_value);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("transport error: {e}"))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("non-2xx status: {}", response.status()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn task_id() -> TaskId {
        TaskId::new()
    }

    #[test]
    fn create_and_get_round_trip() {
        let mut store = PushNotificationStore::new();
        let tid = task_id();
        let cfg = store
            .create(
                tid,
                "https://example.com/webhook".to_owned(),
                "tok".to_owned(),
                None,
            )
            .expect("create must succeed");

        let fetched = store.get(&tid, &cfg.id).expect("must find created config");
        assert_eq!(fetched, &cfg, "round-trip must yield identical config");
    }

    #[test]
    fn create_rejects_non_http_url() {
        let mut store = PushNotificationStore::new();
        let err = store
            .create(
                task_id(),
                "ftp://example.com/x".to_owned(),
                String::new(),
                None,
            )
            .expect_err("non-http url must be rejected");
        assert!(
            matches!(err, Error::InvalidInput { ref reason } if reason.contains("http")),
            "expected InvalidInput about http scheme, got: {err:?}"
        );
    }

    #[test]
    fn create_rejects_malformed_url() {
        let mut store = PushNotificationStore::new();
        let err = store
            .create(task_id(), "not a url".to_owned(), String::new(), None)
            .expect_err("invalid url must be rejected");
        assert!(matches!(err, Error::InvalidInput { .. }));
    }

    #[test]
    fn list_returns_all_for_task() {
        let mut store = PushNotificationStore::new();
        let tid = task_id();
        store
            .create(tid, "https://a/".to_owned(), String::new(), None)
            .unwrap();
        store
            .create(tid, "https://b/".to_owned(), String::new(), None)
            .unwrap();
        // Different task — must not appear.
        store
            .create(task_id(), "https://c/".to_owned(), String::new(), None)
            .unwrap();

        let listed = store.list(&tid);
        assert_eq!(listed.len(), 2, "must list exactly 2 configs for the task");
    }

    #[test]
    fn delete_removes_config_and_returns_true() {
        let mut store = PushNotificationStore::new();
        let tid = task_id();
        let cfg = store
            .create(tid, "https://x/".to_owned(), String::new(), None)
            .unwrap();
        assert!(store.delete(&tid, &cfg.id), "delete must report success");
        assert!(
            store.get(&tid, &cfg.id).is_none(),
            "config must be gone after delete"
        );
        assert!(
            !store.delete(&tid, &cfg.id),
            "second delete must report no-op"
        );
    }

    #[test]
    fn restore_replaces_existing_state() {
        let mut store = PushNotificationStore::new();
        store
            .create(task_id(), "https://a/".to_owned(), String::new(), None)
            .unwrap();

        let tid = task_id();
        let cfg = PushNotificationConfig {
            id: PushNotificationId::new(),
            task_id: tid,
            url: "https://restored/".to_owned(),
            token: String::new(),
            authentication: None,
        };
        store.restore(vec![cfg.clone()]);

        let listed = store.list(&tid);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], cfg);
        assert_eq!(store.all().len(), 1, "previous state must be cleared");
    }

    #[tokio::test]
    async fn deliver_once_succeeds_on_2xx() {
        // Spin up a tiny in-process HTTP listener that returns 200 OK and
        // records the body it received.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(tokio::sync::Mutex::new(Vec::<u8>::new()));
        let received_clone = Arc::clone(&received);
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                if let Ok(n) = sock.read(&mut buf).await {
                    received_clone.lock().await.extend_from_slice(&buf[..n]);
                }
                let _ = sock
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let cfg = PushNotificationConfig {
            id: PushNotificationId::new(),
            task_id: task_id(),
            url: format!("http://{addr}/hook"),
            token: "abc".to_owned(),
            authentication: Some(PushNotificationAuth {
                scheme: "Bearer".to_owned(),
                credentials: "xyz".to_owned(),
            }),
        };
        let client = reqwest::Client::new();
        deliver_once(&client, &cfg, &serde_json::json!({"hi": "there"}))
            .await
            .expect("delivery must succeed");

        let body = received.lock().await;
        let lower = String::from_utf8_lossy(&body).to_lowercase();
        assert!(
            lower.contains("x-trumpet-notification-token: abc"),
            "request must include token header, got:\n{lower}"
        );
        assert!(
            lower.contains("authorization: bearer xyz"),
            "request must include Authorization header, got:\n{lower}"
        );
        assert!(
            lower.contains("\"hi\":\"there\""),
            "body must contain JSON payload, got:\n{lower}"
        );
    }

    #[tokio::test]
    async fn deliver_once_returns_error_on_4xx() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                    .await;
            }
        });

        let cfg = PushNotificationConfig {
            id: PushNotificationId::new(),
            task_id: task_id(),
            url: format!("http://{addr}/hook"),
            token: String::new(),
            authentication: None,
        };
        let client = reqwest::Client::new();
        let err = deliver_once(&client, &cfg, &serde_json::json!({}))
            .await
            .expect_err("4xx must surface as error");
        assert!(err.contains("400"), "error must mention status, got: {err}");
    }
}
