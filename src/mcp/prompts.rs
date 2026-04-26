//! MCP prompts surface for the Trumpet daemon.
//!
//! Prompts are named templates with structured arguments. Trumpet exposes
//! three: `summarize_task`, `assign_task`, and `inspect_conversation`. Each
//! is rendered against the daemon's live state at fetch time, so the LLM
//! receives a fully-resolved prompt rather than a placeholder.

use rmcp::ErrorData as McpError;
use rmcp::model::{
    GetPromptRequestParams, GetPromptResult, ListPromptsResult, Prompt, PromptArgument,
    PromptMessage, PromptMessageContent, PromptMessageRole,
};

use std::collections::HashSet;

use crate::core::types::AgentId;
use crate::core::{ConversationId, TaskId};
use crate::server::AppState;

const PROMPT_SUMMARIZE_TASK: &str = "summarize_task";
const PROMPT_ASSIGN_TASK: &str = "assign_task";
const PROMPT_INSPECT_CONVERSATION: &str = "inspect_conversation";

/// List the prompts the nexus exposes. Pure metadata — no state read.
pub fn list_prompts() -> ListPromptsResult {
    ListPromptsResult {
        prompts: vec![
            Prompt::new(
                PROMPT_SUMMARIZE_TASK,
                Some("Summarise a task's history and current state."),
                Some(vec![
                    PromptArgument::new("task_id")
                        .with_description("UUID of the task to summarise")
                        .with_required(true),
                ]),
            ),
            Prompt::new(
                PROMPT_ASSIGN_TASK,
                Some(
                    "Pick the best agent for a task description, given the registered agent pool.",
                ),
                Some(vec![
                    PromptArgument::new("task_description")
                        .with_description("What the agent must do")
                        .with_required(true),
                    PromptArgument::new("required_tags")
                        .with_description("Comma-separated capability tags the agent must declare")
                        .with_required(false),
                ]),
            ),
            Prompt::new(
                PROMPT_INSPECT_CONVERSATION,
                Some("Render a full conversation thread for review."),
                Some(vec![
                    PromptArgument::new("conversation_id")
                        .with_description("UUID of the conversation")
                        .with_required(true),
                ]),
            ),
        ],
        next_cursor: None,
        meta: None,
    }
}

/// Render a single prompt against [`AppState`] and return the resolved
/// messages. Unknown names → `invalid_params`.
pub async fn get_prompt(
    state: &AppState,
    request: GetPromptRequestParams,
) -> Result<GetPromptResult, McpError> {
    match request.name.as_str() {
        PROMPT_SUMMARIZE_TASK => render_summarize_task(state, &request).await,
        PROMPT_ASSIGN_TASK => render_assign_task(state, &request).await,
        PROMPT_INSPECT_CONVERSATION => render_inspect_conversation(state, &request).await,
        other => Err(McpError::invalid_params(
            format!("unknown prompt: {other}"),
            None,
        )),
    }
}

async fn render_summarize_task(
    state: &AppState,
    request: &GetPromptRequestParams,
) -> Result<GetPromptResult, McpError> {
    let task_id_str = string_arg(request, "task_id")?;
    let task_id = task_id_str
        .parse::<TaskId>()
        .map_err(|_| McpError::invalid_params(format!("invalid task_id: {task_id_str}"), None))?;
    let tasks = state.tasks.read().await;
    let task = tasks
        .get(&task_id)
        .ok_or_else(|| McpError::invalid_params(format!("task not found: {task_id_str}"), None))?;

    let body = serde_json::to_string_pretty(task)
        .map_err(|e| McpError::internal_error(format!("serialise task: {e}"), None))?;

    let user_text = format!(
        "Summarise the following Trumpet task. Cover what was requested, what \
         was produced, and the current state. Highlight blockers and any \
         artifacts the operator should review.\n\nTask:\n{body}"
    );

    Ok(GetPromptResult::new(vec![PromptMessage::new(
        PromptMessageRole::User,
        PromptMessageContent::Text { text: user_text },
    )])
    .with_description(format!("summary request for task {task_id_str}")))
}

async fn render_assign_task(
    state: &AppState,
    request: &GetPromptRequestParams,
) -> Result<GetPromptResult, McpError> {
    let description = string_arg(request, "task_description")?;
    let tags = optional_string_arg(request, "required_tags").unwrap_or_default();

    let registry = state.registry.read().await;
    let agents: Vec<_> = registry.list().into_iter().cloned().collect();
    let agents_json = serde_json::to_string_pretty(&agents)
        .map_err(|e| McpError::internal_error(format!("serialise agents: {e}"), None))?;

    let user_text = format!(
        "We have the following Trumpet agents registered. Choose the single \
         best match for the task below. Reply with the agent's `id` and a one-\
         sentence justification grounded in their capability tags.\n\n\
         Required tags: {tags}\n\nTask:\n{description}\n\nAgents:\n{agents_json}"
    );

    Ok(GetPromptResult::new(vec![PromptMessage::new(
        PromptMessageRole::User,
        PromptMessageContent::Text { text: user_text },
    )])
    .with_description("assignment routing request"))
}

async fn render_inspect_conversation(
    state: &AppState,
    request: &GetPromptRequestParams,
) -> Result<GetPromptResult, McpError> {
    let conv_id_str = string_arg(request, "conversation_id")?;
    let conv_id = conv_id_str.parse::<ConversationId>().map_err(|_| {
        McpError::invalid_params(format!("invalid conversation_id: {conv_id_str}"), None)
    })?;

    let chat = state.chat.read().await;
    let conv = chat.get_conversation(&conv_id).ok_or_else(|| {
        McpError::invalid_params(format!("conversation not found: {conv_id_str}"), None)
    })?;
    let messages = chat
        .get_messages(&conv_id)
        .map_err(|e| McpError::internal_error(format!("messages lookup failed: {e}"), None))?;

    let mut rendered = String::new();
    rendered.push_str(&format!(
        "Conversation: {} (id={})\nParticipants: {}\n\n",
        conv.name.as_deref().unwrap_or("(unnamed)"),
        conv_id,
        format_participants(&conv.participants),
    ));
    for msg in messages {
        rendered.push_str(&format!(
            "[{}] {}: {}\n",
            msg.timestamp.format("%Y-%m-%dT%H:%M:%SZ"),
            msg.sender,
            msg.content
        ));
    }

    Ok(GetPromptResult::new(vec![PromptMessage::new(
        PromptMessageRole::User,
        PromptMessageContent::Text {
            text: format!(
                "Review the following Trumpet conversation thread and \
                 summarise the key decisions, action items, and any \
                 unresolved questions.\n\n{rendered}"
            ),
        },
    )])
    .with_description(format!("conversation inspection for {conv_id_str}")))
}

fn format_participants(participants: &HashSet<AgentId>) -> String {
    if participants.is_empty() {
        return "(none)".to_owned();
    }
    let mut ids: Vec<_> = participants.iter().map(ToString::to_string).collect();
    ids.sort();
    ids.join(", ")
}

fn string_arg(request: &GetPromptRequestParams, name: &str) -> Result<String, McpError> {
    optional_string_arg(request, name)
        .ok_or_else(|| McpError::invalid_params(format!("missing argument: {name}"), None))
}

fn optional_string_arg(request: &GetPromptRequestParams, name: &str) -> Option<String> {
    request
        .arguments
        .as_ref()?
        .get(name)
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
}
