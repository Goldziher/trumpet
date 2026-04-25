//! WebSocket handler for real-time event streaming.
//!
//! Clients connect to `GET /ws` and optionally send a JSON subscription message
//! to filter which event types they receive:
//!
//! ```json
//! {"subscribe": ["agent_registered", "new_message"]}
//! ```
//!
//! If no subscription message is received, all events are forwarded. If the
//! client disconnects or the send buffer is full, the connection is closed.

use std::collections::HashSet;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use serde::Deserialize;
use tokio_stream::wrappers::BroadcastStream;

use super::state::AppState;
use crate::core::Event;

/// JSON message clients send to select which event types to receive.
#[derive(Debug, Deserialize)]
struct SubscribeMessage {
    subscribe: Vec<String>,
}

/// Parse a text WebSocket frame into a topic filter set.
///
/// Returns `None` if the text is not valid JSON or does not match the
/// expected `{"subscribe": [...]}` shape.
fn parse_subscription(text: &str) -> Option<HashSet<String>> {
    serde_json::from_str::<SubscribeMessage>(text)
        .ok()
        .map(|msg| msg.subscribe.into_iter().collect())
}

/// Map an [`Event`] to its canonical topic string used for subscription filtering.
fn event_type(event: &Event) -> &'static str {
    match event {
        Event::AgentRegistered(_) => "agent_registered",
        Event::AgentDeregistered(_) => "agent_deregistered",
        Event::NewMessage(_) => "new_message",
        Event::SkillRegistered(_) => "skill_registered",
        Event::SkillDeregistered(_) => "skill_deregistered",
        Event::TaskCreated(_) => "task_created",
        Event::TaskStatusChanged { .. } => "task_status_changed",
        Event::TaskArtifactAdded { .. } => "task_artifact_added",
    }
}

/// Returns `true` if `event` matches the optional topic filter.
///
/// When `filter` is `None` (client sent no subscription message) every event
/// passes. When `filter` is `Some`, only events whose type string is present
/// in the set pass.
fn matches_filter(event: &Event, filter: Option<&HashSet<String>>) -> bool {
    match filter {
        None => true,
        Some(set) => set.contains(event_type(event)),
    }
}

/// GET /ws — WebSocket stream of domain events.
///
/// Upgrades the HTTP connection to WebSocket, subscribes to the [`MessageBus`],
/// and forwards matching events as JSON text frames. An optional
/// `{"subscribe": [...]}` message from the client enables topic filtering.
///
/// The connection is closed when:
/// - the client disconnects,
/// - a send fails (slow consumer / broken pipe).
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Drive a single WebSocket connection to completion.
async fn handle_socket(socket: WebSocket, state: AppState) {
    let rx = state.bus.subscribe();
    let (mut sink, mut stream) = socket.split();

    // Wait up to one message for a subscription filter before starting the
    // event loop. A non-subscription text message (or any error) is ignored
    // and we fall through with no filter (all events forwarded).
    let mut filter: Option<HashSet<String>> = None;

    // Use a short timeout so clients that never send a subscription message
    // don't block the handler indefinitely before events start flowing.
    let first = tokio::time::timeout(std::time::Duration::from_millis(500), stream.next()).await;

    if let Ok(Some(Ok(Message::Text(text)))) = first
        && let Some(set) = parse_subscription(text.as_str())
    {
        tracing::debug!(topics = ?set, "WebSocket client subscribed to topics");
        filter = Some(set);
    }

    let mut event_stream = BroadcastStream::new(rx);

    loop {
        tokio::select! {
            // Incoming bus event — forward if it passes the filter.
            maybe_event = event_stream.next() => {
                match maybe_event {
                    None => {
                        // Bus closed; shut down cleanly.
                        tracing::debug!("WebSocket event stream ended");
                        break;
                    }
                    Some(Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n))) => {
                        tracing::warn!(dropped = n, "WebSocket subscriber lagged, events lost");
                        // Continue — don't disconnect on lag.
                    }
                    Some(Ok(event)) => {
                        if !matches_filter(&event, filter.as_ref()) {
                            continue;
                        }
                        match serde_json::to_string(&event) {
                            Err(e) => {
                                tracing::error!(error = %e, "failed to serialize event for WebSocket");
                            }
                            Ok(json) => {
                                if sink.send(Message::Text(json.into())).await.is_err() {
                                    tracing::warn!("WebSocket send failed, closing connection");
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            // Client-side message (ping, close, further control frames).
            maybe_msg = stream.next() => {
                match maybe_msg {
                    None | Some(Err(_)) => {
                        tracing::debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::debug!("WebSocket client sent close frame");
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                    Some(Ok(_)) => {
                        // Ignore pings, binary frames, and additional text frames.
                    }
                }
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{
        AgentId, AgentInfo, AgentStatus, ChatMessage, ConversationId, MessageId,
    };
    use chrono::Utc;

    fn make_agent_event() -> Event {
        Event::AgentRegistered(AgentInfo {
            id: AgentId::new(),
            name: "test-agent".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        })
    }

    fn make_message_event() -> Event {
        Event::NewMessage(ChatMessage {
            id: MessageId::new(),
            conversation_id: ConversationId::new(),
            sender: AgentId::new(),
            content: "hello".to_owned(),
            timestamp: Utc::now(),
        })
    }

    #[test]
    fn subscription_filter_accepts_matching_event() {
        let filter: HashSet<String> = ["agent_registered".to_owned()].into_iter().collect();
        let event = make_agent_event();
        assert!(
            matches_filter(&event, Some(&filter)),
            "filter containing 'agent_registered' must accept an AgentRegistered event"
        );
    }

    #[test]
    fn subscription_filter_rejects_non_matching_event() {
        let filter: HashSet<String> = ["agent_registered".to_owned()].into_iter().collect();
        let event = make_message_event();
        assert!(
            !matches_filter(&event, Some(&filter)),
            "filter containing only 'agent_registered' must reject a NewMessage event"
        );
    }

    #[test]
    fn empty_filter_accepts_all_events() {
        let agent_event = make_agent_event();
        let message_event = make_message_event();
        assert!(
            matches_filter(&agent_event, None),
            "None filter must accept AgentRegistered event"
        );
        assert!(
            matches_filter(&message_event, None),
            "None filter must accept NewMessage event"
        );
    }

    #[test]
    fn parse_subscription_message_valid() {
        let json = r#"{"subscribe": ["agent_registered"]}"#;
        let result = parse_subscription(json);
        assert!(
            result.is_some(),
            "valid subscription JSON must parse successfully"
        );
        let set = result.unwrap();
        assert!(
            set.contains("agent_registered"),
            "parsed set must contain 'agent_registered', got: {set:?}"
        );
    }

    #[test]
    fn parse_subscription_message_invalid_json_returns_none() {
        let garbage = "not valid json at all !!!";
        let result = parse_subscription(garbage);
        assert!(
            result.is_none(),
            "garbage input must return None, got: {result:?}"
        );
    }
}
