//! MCP resources surface for the Trumpet daemon.
//!
//! Resources are read-only views of nexus state, addressed by `trumpet://`
//! URIs. Each handler reads from [`AppState`] and returns the requested slice
//! as JSON. Listing and reading are paginated only at the protocol level —
//! Trumpet currently returns the full set in one page.
//!
//! ## URIs
//!
//! | URI                                         | Returns                          |
//! |---------------------------------------------|----------------------------------|
//! | `trumpet://agents`                          | `Vec<AgentInfo>`                 |
//! | `trumpet://agents/{id}`                     | single `AgentInfo`               |
//! | `trumpet://conversations`                   | `Vec<Conversation>`              |
//! | `trumpet://conversations/{id}`              | conversation + full message log |
//! | `trumpet://tasks`                           | `Vec<Task>`                      |
//! | `trumpet://tasks/{id}`                     | single `Task` with artifacts    |
//! | `trumpet://tools`                           | `Vec<ToolInfo>`                  |

use rmcp::ErrorData as McpError;
use rmcp::model::{
    ListResourcesResult, RawResource, ReadResourceRequestParams, ReadResourceResult, Resource,
    ResourceContents,
};

use crate::core::types::AgentId;
use crate::core::{ConversationId, TaskId};
use crate::server::AppState;

const URI_AGENTS: &str = "trumpet://agents";
const URI_CONVERSATIONS: &str = "trumpet://conversations";
const URI_TASKS: &str = "trumpet://tasks";
const URI_TOOLS: &str = "trumpet://tools";

/// Annotation-free resource constructor — rmcp's `Resource` is
/// `Annotated<RawResource>`, so we wrap the raw struct.
fn make_resource(uri: &str, name: &str, description: &str) -> Resource {
    use rmcp::model::AnnotateAble;
    let raw = RawResource {
        uri: uri.to_owned(),
        name: name.to_owned(),
        title: None,
        description: Some(description.to_owned()),
        mime_type: Some("application/json".to_owned()),
        size: None,
        icons: None,
        meta: None,
    };
    raw.no_annotation()
}

/// List the static collection-level resources exposed by the nexus.
///
/// Per-item resources (`trumpet://agents/{id}` etc.) are not enumerated
/// here — clients fetch them by URI when they need them. This matches the
/// MCP spec, which allows servers to expose resource templates separately
/// when there are many or unbounded items.
pub fn list_resources() -> ListResourcesResult {
    ListResourcesResult {
        resources: vec![
            make_resource(URI_AGENTS, "agents", "All registered agents"),
            make_resource(
                URI_CONVERSATIONS,
                "conversations",
                "All conversations with their participants",
            ),
            make_resource(URI_TASKS, "tasks", "All A2A tasks (any status)"),
            make_resource(URI_TOOLS, "tools", "Tool registry contents"),
        ],
        next_cursor: None,
        meta: None,
    }
}

/// Resolve a `trumpet://` URI against [`AppState`] and produce the
/// matching MCP `ReadResourceResult`.
///
/// Unknown URIs return [`McpError::invalid_params`] so the client gets a
/// clean 400-style failure rather than an opaque internal error.
pub async fn read_resource(
    state: &AppState,
    request: ReadResourceRequestParams,
) -> Result<ReadResourceResult, McpError> {
    let uri = request.uri.as_str();

    let json = match uri {
        URI_AGENTS => {
            let registry = state.registry.read().await;
            let agents: Vec<_> = registry.list().into_iter().cloned().collect();
            serde_json::to_string_pretty(&agents)
        }
        URI_CONVERSATIONS => {
            let chat = state.chat.read().await;
            let convs: Vec<_> = chat.list_conversations().into_iter().cloned().collect();
            serde_json::to_string_pretty(&convs)
        }
        URI_TASKS => {
            let tasks = state.tasks.read().await;
            let all: Vec<_> = tasks.list().into_iter().cloned().collect();
            serde_json::to_string_pretty(&all)
        }
        URI_TOOLS => {
            let tools = state.tools.read().await;
            let all: Vec<_> = tools.list().into_iter().cloned().collect();
            serde_json::to_string_pretty(&all)
        }
        other => {
            if let Some(rest) = other.strip_prefix("trumpet://agents/") {
                let id = parse_agent_id(rest)?;
                let registry = state.registry.read().await;
                let info = registry
                    .get(&id)
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("agent not found: {rest}"), None)
                    })?
                    .clone();
                serde_json::to_string_pretty(&info)
            } else if let Some(rest) = other.strip_prefix("trumpet://conversations/") {
                let id = parse_conversation_id(rest)?;
                let chat = state.chat.read().await;
                let conv = chat.get_conversation(&id).ok_or_else(|| {
                    McpError::invalid_params(format!("conversation not found: {rest}"), None)
                })?;
                let messages = chat.get_messages(&id).map_err(|e| {
                    McpError::internal_error(format!("messages lookup failed: {e}"), None)
                })?;
                serde_json::to_string_pretty(&serde_json::json!({
                    "conversation": conv,
                    "messages": messages,
                }))
            } else if let Some(rest) = other.strip_prefix("trumpet://tasks/") {
                let id = parse_task_id(rest)?;
                let tasks = state.tasks.read().await;
                let task = tasks
                    .get(&id)
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("task not found: {rest}"), None)
                    })?
                    .clone();
                serde_json::to_string_pretty(&task)
            } else {
                return Err(McpError::invalid_params(
                    format!("unknown resource URI: {other}"),
                    None,
                ));
            }
        }
    }
    .map_err(|e| McpError::internal_error(format!("serialise resource: {e}"), None))?;

    Ok(ReadResourceResult::new(vec![
        ResourceContents::TextResourceContents {
            uri: uri.to_owned(),
            mime_type: Some("application/json".to_owned()),
            text: json,
            meta: None,
        },
    ]))
}

fn parse_agent_id(s: &str) -> Result<AgentId, McpError> {
    s.parse::<AgentId>()
        .map_err(|_| McpError::invalid_params(format!("invalid agent id: {s}"), None))
}

fn parse_conversation_id(s: &str) -> Result<ConversationId, McpError> {
    s.parse::<ConversationId>()
        .map_err(|_| McpError::invalid_params(format!("invalid conversation id: {s}"), None))
}

fn parse_task_id(s: &str) -> Result<TaskId, McpError> {
    s.parse::<TaskId>()
        .map_err(|_| McpError::invalid_params(format!("invalid task id: {s}"), None))
}
