//! Axum route handlers for the Trumpet HTTP API.

use std::convert::Infallible;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::response::sse::{Event as SseEvent, Sse};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::core::Event;
use crate::core::types::{AgentId, AgentInfo, ChatMessage, Conversation, ConversationId};
use crate::error::{Error, Result};

use super::state::AppState;

// ── Request / response types ──────────────────────────────────────────────────

/// Request body for agent registration.
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
}

/// Request body for agent deregistration.
#[derive(Debug, Deserialize)]
pub struct DeregisterRequest {
    pub agent_id: AgentId,
}

/// Request body for creating a new conversation.
#[derive(Debug, Deserialize)]
pub struct CreateConversationRequest {
    pub name: Option<String>,
    pub participants: Vec<AgentId>,
}

/// Request body for sending a message into a conversation.
#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub sender: AgentId,
    pub content: String,
}

/// Response body for the health-check endpoint.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Parse a [`ConversationId`] from a path segment string.
fn parse_conversation_id(raw: &str) -> Result<ConversationId> {
    raw.parse::<ConversationId>()
        .map_err(|_| Error::ConversationNotFound { id: raw.to_owned() })
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /health — liveness check.
async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

/// POST /agents/register — register a new agent.
async fn register_agent(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<AgentInfo>> {
    let mut registry = state.registry.write().await;
    let info = registry.register(&req.name)?;
    Ok(Json(info))
}

/// POST /agents/deregister — deregister an existing agent.
async fn deregister_agent(
    State(state): State<AppState>,
    Json(req): Json<DeregisterRequest>,
) -> Result<Json<AgentInfo>> {
    let mut registry = state.registry.write().await;
    let info = registry.deregister(&req.agent_id)?;
    Ok(Json(info))
}

/// GET /agents — list all registered agents.
async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentInfo>> {
    let registry = state.registry.read().await;
    let agents: Vec<AgentInfo> = registry.list().into_iter().cloned().collect();
    Json(agents)
}

/// POST /conversations — create a new conversation.
async fn create_conversation(
    State(state): State<AppState>,
    Json(req): Json<CreateConversationRequest>,
) -> Result<Json<Conversation>> {
    let mut chat = state.chat.write().await;
    let conv = chat.create_conversation(req.name, req.participants)?;
    Ok(Json(conv))
}

/// GET /conversations/{id} — fetch a single conversation.
async fn get_conversation(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Conversation>> {
    let conv_id = parse_conversation_id(&id)?;
    let chat = state.chat.read().await;
    let conv = chat
        .get_conversation(&conv_id)
        .ok_or(Error::ConversationNotFound { id })?
        .clone();
    Ok(Json(conv))
}

/// POST /conversations/{id}/messages — send a message.
async fn send_message(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<ChatMessage>> {
    let conv_id = parse_conversation_id(&id)?;
    let mut chat = state.chat.write().await;
    let msg = chat.send_message(&conv_id, req.sender, req.content)?;
    Ok(Json(msg))
}

/// GET /conversations/{id}/messages — retrieve message history.
async fn get_messages(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<ChatMessage>>> {
    let conv_id = parse_conversation_id(&id)?;
    let chat = state.chat.read().await;
    let messages = chat.get_messages(&conv_id)?.to_vec();
    Ok(Json(messages))
}

/// GET /events — SSE stream of domain events.
///
/// Serializes each [`Event`] variant into a typed SSE event. Lagged
/// subscribers (those that fall more than the bus capacity behind) are
/// silently dropped from the stream.
async fn events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<SseEvent, Infallible>>> {
    let rx = state.bus.subscribe();
    let stream = BroadcastStream::new(rx)
        .filter_map(|result| match result {
            Ok(event) => Some(event),
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
                tracing::warn!(dropped = n, "SSE subscriber lagged, events lost");
                None
            }
        })
        .filter_map(|event| match serde_json::to_string(&event) {
            Ok(data) => Some(Ok(SseEvent::default().event(event_type(&event)).data(data))),
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize event for SSE");
                None
            }
        });
    Sse::new(stream)
}

/// Extract the SSE event type string from an [`Event`] variant.
fn event_type(event: &Event) -> &'static str {
    match event {
        Event::AgentRegistered(_) => "agent_registered",
        Event::AgentDeregistered(_) => "agent_deregistered",
        Event::NewMessage(_) => "new_message",
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Build the axum [`Router`] with all routes wired up and `state` attached.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/agents", get(list_agents))
        .route("/agents/register", post(register_agent))
        .route("/agents/deregister", post(deregister_agent))
        .route("/conversations", post(create_conversation))
        .route("/conversations/{id}", get(get_conversation))
        .route(
            "/conversations/{id}/messages",
            post(send_message).get(get_messages),
        )
        .route("/events", get(events))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_conversation_id_valid_uuid_succeeds() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let result = parse_conversation_id(uuid_str);
        assert!(
            result.is_ok(),
            "valid UUID string must parse to ConversationId, got: {result:?}"
        );
    }

    #[test]
    fn parse_conversation_id_invalid_string_returns_not_found_error() {
        let result = parse_conversation_id("not-a-uuid");
        assert!(
            matches!(result, Err(Error::ConversationNotFound { ref id }) if id == "not-a-uuid"),
            "invalid UUID must produce ConversationNotFound, got: {result:?}"
        );
    }

    #[test]
    fn event_type_agent_registered() {
        use crate::core::types::{AgentId, AgentInfo, AgentStatus};
        use chrono::Utc;

        let info = AgentInfo {
            id: AgentId::new(),
            name: "test".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        };
        assert_eq!(
            event_type(&Event::AgentRegistered(info)),
            "agent_registered"
        );
    }

    #[test]
    fn event_type_agent_deregistered() {
        assert_eq!(
            event_type(&Event::AgentDeregistered(AgentId::new())),
            "agent_deregistered"
        );
    }

    #[test]
    fn event_type_new_message() {
        use crate::core::types::{AgentId, ChatMessage, ConversationId, MessageId};
        use chrono::Utc;

        let msg = ChatMessage {
            id: MessageId::new(),
            conversation_id: ConversationId::new(),
            sender: AgentId::new(),
            content: "hello".to_owned(),
            timestamp: Utc::now(),
        };
        assert_eq!(event_type(&Event::NewMessage(msg)), "new_message");
    }

    #[test]
    fn event_serializes_to_json() {
        use crate::core::types::{AgentId, AgentInfo, AgentStatus};
        use chrono::Utc;

        let info = AgentInfo {
            id: AgentId::new(),
            name: "test".to_owned(),
            registered_at: Utc::now(),
            status: AgentStatus::Connected,
        };
        let event = Event::AgentRegistered(info);
        let json = serde_json::to_string(&event).expect("Event must serialize");
        assert!(
            json.contains("agent_registered"),
            "JSON must contain event type tag"
        );
    }
}
