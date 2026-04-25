//! Axum route handlers for the Trumpet HTTP API.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event as SseEvent, Sse};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt as _;
use tokio_stream::wrappers::BroadcastStream;

use crate::core::task_types::{MessageRole, Part, TaskMessage};
use crate::core::types::{
    AgentId, AgentInfo, ChatMessage, Conversation, ConversationId, MessageId, ToolInfo,
};
use crate::core::{ContextId, DefaultTaskRouter, Task, TaskFacade, TaskFilter, TaskId, TaskState};
use crate::error::{Error, Result};

use super::state::AppState;
use super::ws;

// ── Request / response types ──────────────────────────────────────────────────

/// Request body for agent registration.
#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub name: String,
    /// Optional agent capabilities (ADR-015).
    #[serde(default)]
    pub capabilities: Option<crate::core::AgentCapabilities>,
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

/// Request body for invoking a tool.
#[derive(Debug, Deserialize)]
pub struct InvokeToolRequest {
    /// The JSON payload to pass to the tool.
    pub input: serde_json::Value,
}

/// Response body for the health-check endpoint.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

/// Request body for submitting a new task.
#[derive(Debug, Deserialize)]
pub struct SubmitTaskRequest {
    pub message: String,
    pub context_id: Option<String>,
    pub assignee: Option<String>,
}

/// Query parameters for listing tasks.
#[derive(Debug, Deserialize)]
pub struct ListTasksQuery {
    pub context_id: Option<String>,
    pub state: Option<String>,
    pub assignee: Option<String>,
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
    let info = registry.register(&req.name, req.capabilities)?;
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

/// GET /conversations — list all conversations.
async fn list_conversations(State(state): State<AppState>) -> Json<Vec<Conversation>> {
    let chat = state.chat.read().await;
    let convs: Vec<Conversation> = chat.list_conversations().into_iter().cloned().collect();
    Json(convs)
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

/// GET /tools — list all registered tools.
async fn list_tools(State(state): State<AppState>) -> Json<Vec<ToolInfo>> {
    let tools = state.tools.read().await;
    let list: Vec<ToolInfo> = tools.list().into_iter().cloned().collect();
    Json(list)
}

/// GET /tools/{name} — find a tool by name.
async fn get_tool_by_name(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<ToolInfo>> {
    let tools = state.tools.read().await;
    let info = tools
        .find_by_name(&name)
        .ok_or_else(|| Error::ToolNotFound { name: name.clone() })?
        .clone();
    Ok(Json(info))
}

/// POST /tools/{name}/invoke — invoke a tool by name.
///
/// Resolves the tool from the registry and dispatches to the appropriate
/// provider. Built-in tools return immediately; agent-provided tools
/// create a task.
async fn invoke_tool(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<InvokeToolRequest>,
) -> Result<Json<serde_json::Value>> {
    let invoker = crate::core::ToolInvoker::new(
        Arc::clone(&state.tools),
        Arc::clone(&state.code_tools),
        Arc::clone(&state.tasks),
        Arc::clone(&state.registry),
    );
    let result = invoker.invoke(&name, req.input).await?;
    let json = serde_json::to_value(result).map_err(|e| Error::ToolInvocationFailed {
        name,
        reason: format!("serialization failed: {e}"),
    })?;
    Ok(Json(json))
}

// ── Task helpers ─────────────────────────────────────────────────────────────

fn make_facade(state: &AppState) -> TaskFacade {
    TaskFacade::new(
        Arc::clone(&state.tasks),
        Arc::clone(&state.registry),
        Box::new(DefaultTaskRouter),
    )
}

/// POST /tasks — submit a new task.
async fn submit_task(
    State(state): State<AppState>,
    Json(req): Json<SubmitTaskRequest>,
) -> Result<Json<Task>> {
    let context_id = req
        .context_id
        .as_deref()
        .map(|s| {
            s.parse::<ContextId>()
                .map_err(|_| Error::ConfigValidationFailed {
                    reason: format!("invalid context_id UUID: '{s}'"),
                })
        })
        .transpose()?;

    let assignee = req
        .assignee
        .as_deref()
        .map(|s| {
            s.parse::<AgentId>()
                .map_err(|_| Error::ConfigValidationFailed {
                    reason: format!("invalid assignee UUID: '{s}'"),
                })
        })
        .transpose()?;

    let message = TaskMessage {
        id: MessageId::new(),
        role: MessageRole::User,
        parts: vec![Part::Text { text: req.message }],
        metadata: None,
    };

    let facade = make_facade(&state);
    let task = facade
        .submit_task(message, context_id, assignee, None)
        .await?;
    Ok(Json(task))
}

/// GET /tasks/{id} — get a task by ID.
async fn get_task_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>> {
    let task_id = id
        .parse::<TaskId>()
        .map_err(|_| Error::TaskNotFound { id: id.clone() })?;
    let facade = make_facade(&state);
    let task = facade.get_task(&task_id).await?;
    Ok(Json(task))
}

/// GET /tasks — list tasks with optional filters.
async fn list_tasks_handler(
    State(state): State<AppState>,
    Query(query): Query<ListTasksQuery>,
) -> Result<Json<Vec<Task>>> {
    let context_id = query
        .context_id
        .as_deref()
        .map(|s| {
            s.parse::<ContextId>()
                .map_err(|_| Error::ConfigValidationFailed {
                    reason: format!("invalid context_id UUID: '{s}'"),
                })
        })
        .transpose()?;

    let state_filter = query
        .state
        .as_deref()
        .map(|s| {
            s.parse::<TaskState>()
                .map_err(|_| Error::ConfigValidationFailed {
                    reason: format!("'{s}' is not a valid task state"),
                })
        })
        .transpose()?;

    let assignee = query
        .assignee
        .as_deref()
        .map(|s| {
            s.parse::<AgentId>()
                .map_err(|_| Error::ConfigValidationFailed {
                    reason: format!("invalid assignee UUID: '{s}'"),
                })
        })
        .transpose()?;

    let filter = TaskFilter {
        context_id,
        state: state_filter,
        assignee,
    };
    let facade = make_facade(&state);
    let tasks = facade.list_tasks(&filter).await;
    Ok(Json(tasks))
}

/// POST /tasks/{id}/cancel — cancel a task.
async fn cancel_task_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Task>> {
    let task_id = id
        .parse::<TaskId>()
        .map_err(|_| Error::TaskNotFound { id: id.clone() })?;
    let facade = make_facade(&state);
    let task = facade.cancel_task(&task_id, None).await?;
    Ok(Json(task))
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
            Ok(data) => Some(Ok(SseEvent::default().event(event.event_type()).data(data))),
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize event for SSE");
                None
            }
        });
    Sse::new(stream)
}

// event_type is provided by Event::event_type() on crate::core::bus::Event.

// ── Router ────────────────────────────────────────────────────────────────────

/// Build the axum [`Router`] with all routes wired up and `state` attached.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/agents", get(list_agents))
        .route("/agents/register", post(register_agent))
        .route("/agents/deregister", post(deregister_agent))
        .route(
            "/conversations",
            post(create_conversation).get(list_conversations),
        )
        .route("/conversations/{id}", get(get_conversation))
        .route(
            "/conversations/{id}/messages",
            post(send_message).get(get_messages),
        )
        .route("/tools", get(list_tools))
        .route("/tools/{name}", get(get_tool_by_name))
        .route("/tools/{name}/invoke", post(invoke_tool))
        .route("/tasks", post(submit_task).get(list_tasks_handler))
        .route("/tasks/{id}", get(get_task_by_id))
        .route("/tasks/{id}/cancel", post(cancel_task_handler))
        .route("/events", get(events))
        .route("/ws", get(ws::ws_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Event;

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
            capabilities: None,
        };
        assert_eq!(
            Event::AgentRegistered(info).event_type(),
            "agent_registered"
        );
    }

    #[test]
    fn event_type_agent_deregistered() {
        assert_eq!(
            Event::AgentDeregistered(AgentId::new()).event_type(),
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
        assert_eq!(Event::NewMessage(msg).event_type(), "new_message");
    }

    #[test]
    fn event_type_tool_registered() {
        use crate::core::types::{ToolId, ToolInfo, ToolProvider};

        let info = ToolInfo {
            id: ToolId::new(),
            name: "test".to_owned(),
            description: "desc".to_owned(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            provider: ToolProvider::BuiltIn,
        };
        assert_eq!(Event::ToolRegistered(info).event_type(), "tool_registered");
    }

    #[test]
    fn event_type_tool_deregistered() {
        use crate::core::types::ToolId;

        assert_eq!(
            Event::ToolDeregistered(ToolId::new()).event_type(),
            "tool_deregistered"
        );
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
            capabilities: None,
        };
        let event = Event::AgentRegistered(info);
        let json = serde_json::to_string(&event).expect("Event must serialize");
        assert!(
            json.contains("agent_registered"),
            "JSON must contain event type tag"
        );
    }
}
