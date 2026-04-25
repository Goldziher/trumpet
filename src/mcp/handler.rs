//! MCP server handler for the Trumpet agent nexus.
//!
//! [`TrumpetMcpServer`] exposes tools for managing agents, tools, and
//! conversations over the Model Context Protocol.

use rmcp::{
    ErrorData as McpError, ServerHandler, handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters, model::*, schemars, tool, tool_handler, tool_router,
};

use crate::server::AppState;

// ── Argument types ────────────────────────────────────────────────────────────

/// Arguments for the `register_agent` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RegisterAgentArgs {
    /// Human-readable name for the agent (e.g. `"claude-code-1"`).
    pub name: String,
}

/// Arguments for the `deregister_agent` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeregisterAgentArgs {
    /// UUID string of the agent to deregister.
    pub agent_id: String,
}

/// Arguments for the `create_conversation` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateConversationArgs {
    /// Optional display name for the conversation.
    pub name: Option<String>,
    /// UUID strings of agents participating in the conversation.
    pub participants: Vec<String>,
}

/// Arguments for the `send_message` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SendMessageArgs {
    /// UUID string of the target conversation.
    pub conversation_id: String,
    /// UUID string of the sending agent.
    pub sender_agent_id: String,
    /// Message body.
    pub content: String,
}

/// Arguments for the `get_messages` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetMessagesArgs {
    /// UUID string of the conversation to retrieve messages from.
    pub conversation_id: String,
}

/// Arguments for the `submit_task` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SubmitTaskArgs {
    /// Human-readable task description.
    pub message: String,
    /// Optional context (session) UUID string.
    pub context_id: Option<String>,
    /// Optional UUID string of the agent to assign the task to.
    pub assignee: Option<String>,
}

/// Arguments for the `get_task` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetTaskArgs {
    /// UUID string of the task to retrieve.
    pub task_id: String,
}

/// Arguments for the `list_tasks` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListTasksArgs {
    /// Filter by context UUID string.
    pub context_id: Option<String>,
    /// Filter by task state (e.g. `"submitted"`, `"working"`).
    pub state: Option<String>,
    /// Filter by assignee agent UUID string.
    pub assignee: Option<String>,
}

/// Arguments for the `cancel_task` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CancelTaskArgs {
    /// UUID string of the task to cancel.
    pub task_id: String,
    /// Optional reason for cancellation.
    pub reason: Option<String>,
}

// ── Server struct ─────────────────────────────────────────────────────────────

/// MCP server that exposes Trumpet's agent nexus capabilities as tools.
#[derive(Clone)]
pub struct TrumpetMcpServer {
    state: AppState,
    tool_router: ToolRouter<TrumpetMcpServer>,
}

impl TrumpetMcpServer {
    /// Create a new [`TrumpetMcpServer`] backed by the given shared state.
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

// ── Tool implementations ──────────────────────────────────────────────────────

#[tool_router]
impl TrumpetMcpServer {
    /// List all currently registered agents.
    #[tool(description = "List all agents currently registered with the Trumpet nexus.")]
    async fn list_agents(&self) -> Result<CallToolResult, McpError> {
        let registry = self.state.registry.read().await;
        let agents: Vec<_> = registry.list().into_iter().cloned().collect();
        let json = serde_json::to_string(&agents)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Register a new agent by name.
    #[tool(description = "Register a new agent with the Trumpet nexus and return its AgentInfo.")]
    async fn register_agent(
        &self,
        Parameters(args): Parameters<RegisterAgentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let info = {
            let mut registry = self.state.registry.write().await;
            registry
                .register(&args.name, None)
                .map_err(|e| McpError::invalid_params(e.to_string(), None))?
        };
        let json = serde_json::to_string(&info)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Deregister an agent by ID.
    #[tool(description = "Deregister an agent from the Trumpet nexus.")]
    async fn deregister_agent(
        &self,
        Parameters(args): Parameters<DeregisterAgentArgs>,
    ) -> Result<CallToolResult, McpError> {
        let agent_id = args
            .agent_id
            .parse::<crate::core::types::AgentId>()
            .map_err(|e| McpError::invalid_params(format!("invalid agent_id: {e}"), None))?;

        let info = {
            let mut registry = self.state.registry.write().await;
            registry
                .deregister(&agent_id)
                .map_err(|e| McpError::invalid_params(e.to_string(), None))?
        };
        let json = serde_json::to_string(&info)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Create a new conversation between agents.
    #[tool(description = "Create a new conversation with the specified participants.")]
    async fn create_conversation(
        &self,
        Parameters(args): Parameters<CreateConversationArgs>,
    ) -> Result<CallToolResult, McpError> {
        let participants: Vec<crate::core::types::AgentId> = args
            .participants
            .iter()
            .map(|s| {
                s.parse::<crate::core::types::AgentId>().map_err(|e| {
                    McpError::invalid_params(format!("invalid agent_id '{s}': {e}"), None)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let conv = {
            let mut chat = self.state.chat.write().await;
            chat.create_conversation(args.name, participants)
                .map_err(|e| McpError::invalid_params(e.to_string(), None))?
        };
        let json = serde_json::to_string(&conv)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List all registered tools.
    #[tool(description = "List all tools registered with the Trumpet nexus.")]
    async fn list_tools(&self) -> Result<CallToolResult, McpError> {
        let tools = self.state.tools.read().await;
        let tool_list: Vec<_> = tools.list().into_iter().cloned().collect();
        let json = serde_json::to_string(&tool_list)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List all active conversations.
    #[tool(description = "List all active conversations in the Trumpet nexus.")]
    async fn list_conversations(&self) -> Result<CallToolResult, McpError> {
        let chat = self.state.chat.read().await;
        let convs: Vec<_> = chat.list_conversations().into_iter().cloned().collect();
        let json = serde_json::to_string(&convs)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Send a message into an existing conversation.
    #[tool(
        description = "Send a message from an agent into an existing conversation. Returns the stored ChatMessage."
    )]
    async fn send_message(
        &self,
        Parameters(args): Parameters<SendMessageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let conversation_id = args
            .conversation_id
            .parse::<crate::core::types::ConversationId>()
            .map_err(|e| McpError::invalid_params(format!("invalid conversation_id: {e}"), None))?;

        let sender = args
            .sender_agent_id
            .parse::<crate::core::types::AgentId>()
            .map_err(|e| McpError::invalid_params(format!("invalid sender_agent_id: {e}"), None))?;

        let msg = {
            let mut chat = self.state.chat.write().await;
            chat.send_message(&conversation_id, sender, args.content)
                .map_err(|e| McpError::invalid_params(e.to_string(), None))?
        };

        let json = serde_json::to_string(&msg)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Retrieve message history for a conversation.
    #[tool(description = "Get the ordered message history for a conversation.")]
    async fn get_messages(
        &self,
        Parameters(args): Parameters<GetMessagesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let conversation_id = args
            .conversation_id
            .parse::<crate::core::types::ConversationId>()
            .map_err(|e| McpError::invalid_params(format!("invalid conversation_id: {e}"), None))?;

        let chat = self.state.chat.read().await;
        let messages = chat
            .get_messages(&conversation_id)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        let json = serde_json::to_string(messages)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Submit a new task.
    #[tool(description = "Submit a new task to the Trumpet nexus. Returns the created Task.")]
    async fn submit_task(
        &self,
        Parameters(args): Parameters<SubmitTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        use crate::core::task_types::{MessageRole, Part, TaskMessage};
        use crate::core::types::MessageId;

        let context_id = args
            .context_id
            .as_deref()
            .map(|s| {
                s.parse::<crate::core::ContextId>()
                    .map_err(|e| McpError::invalid_params(format!("invalid context_id: {e}"), None))
            })
            .transpose()?;

        let assignee = args
            .assignee
            .as_deref()
            .map(|s| {
                s.parse::<crate::core::types::AgentId>()
                    .map_err(|e| McpError::invalid_params(format!("invalid assignee: {e}"), None))
            })
            .transpose()?;

        let message = TaskMessage {
            id: MessageId::new(),
            role: MessageRole::User,
            parts: vec![Part::Text { text: args.message }],
            metadata: None,
        };

        let facade = crate::core::TaskFacade::new(
            std::sync::Arc::clone(&self.state.tasks),
            std::sync::Arc::clone(&self.state.registry),
            Box::new(crate::core::DefaultTaskRouter),
        );
        let task = facade
            .submit_task(message, context_id, assignee, None)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let json = serde_json::to_string(&task)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Get a task by ID.
    #[tool(description = "Retrieve a task by its UUID.")]
    async fn get_task(
        &self,
        Parameters(args): Parameters<GetTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let task_id = args
            .task_id
            .parse::<crate::core::TaskId>()
            .map_err(|e| McpError::invalid_params(format!("invalid task_id: {e}"), None))?;

        let facade = crate::core::TaskFacade::new(
            std::sync::Arc::clone(&self.state.tasks),
            std::sync::Arc::clone(&self.state.registry),
            Box::new(crate::core::DefaultTaskRouter),
        );
        let task = facade.get_task(&task_id).await.map_err(|e| match &e {
            crate::error::Error::TaskNotFound { .. } => {
                McpError::invalid_params(e.to_string(), None)
            }
            _ => McpError::internal_error(e.to_string(), None),
        })?;

        let json = serde_json::to_string(&task)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// List tasks with optional filters.
    #[tool(description = "List tasks, optionally filtered by context_id, state, or assignee.")]
    async fn list_tasks(
        &self,
        Parameters(args): Parameters<ListTasksArgs>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = args
            .context_id
            .as_deref()
            .map(|s| {
                s.parse::<crate::core::ContextId>()
                    .map_err(|e| McpError::invalid_params(format!("invalid context_id: {e}"), None))
            })
            .transpose()?;

        let state_filter = args
            .state
            .as_deref()
            .map(|s| {
                s.parse::<crate::core::TaskState>()
                    .map_err(|_| McpError::invalid_params(format!("invalid state: '{s}'"), None))
            })
            .transpose()?;

        let assignee = args
            .assignee
            .as_deref()
            .map(|s| {
                s.parse::<crate::core::types::AgentId>()
                    .map_err(|e| McpError::invalid_params(format!("invalid assignee: {e}"), None))
            })
            .transpose()?;

        let filter = crate::core::TaskFilter {
            context_id,
            state: state_filter,
            assignee,
        };
        let facade = crate::core::TaskFacade::new(
            std::sync::Arc::clone(&self.state.tasks),
            std::sync::Arc::clone(&self.state.registry),
            Box::new(crate::core::DefaultTaskRouter),
        );
        let tasks = facade.list_tasks(&filter).await;

        let json = serde_json::to_string(&tasks)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    /// Cancel a task.
    #[tool(description = "Cancel a task by its UUID.")]
    async fn cancel_task(
        &self,
        Parameters(args): Parameters<CancelTaskArgs>,
    ) -> Result<CallToolResult, McpError> {
        let task_id = args
            .task_id
            .parse::<crate::core::TaskId>()
            .map_err(|e| McpError::invalid_params(format!("invalid task_id: {e}"), None))?;

        let facade = crate::core::TaskFacade::new(
            std::sync::Arc::clone(&self.state.tasks),
            std::sync::Arc::clone(&self.state.registry),
            Box::new(crate::core::DefaultTaskRouter),
        );
        let task = facade
            .cancel_task(&task_id, None)
            .await
            .map_err(|e| match &e {
                crate::error::Error::TaskNotFound { .. } => {
                    McpError::invalid_params(e.to_string(), None)
                }
                _ => McpError::internal_error(e.to_string(), None),
            })?;

        let json = serde_json::to_string(&task)
            .map_err(|e| McpError::internal_error(format!("serialization failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![Content::text(json)]))
    }
}

// ── ServerHandler ─────────────────────────────────────────────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TrumpetMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("trumpet", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions("Trumpet agent nexus — manage agents, tools, and conversations.")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn make_server() -> TrumpetMcpServer {
        TrumpetMcpServer::new(AppState::new(Config::default()))
    }

    #[tokio::test]
    async fn list_agents_tool_returns_empty_initially() {
        let server = make_server();
        let result = server
            .list_agents()
            .await
            .expect("list_agents must succeed");

        assert!(
            !result.is_error.unwrap_or(false),
            "result must not be an error"
        );
        let content = result.content.first().expect("result must have content");
        let text = content
            .as_text()
            .expect("content must be text")
            .text
            .as_str();
        let agents: Vec<serde_json::Value> =
            serde_json::from_str(text).expect("text must be valid JSON array");
        assert!(
            agents.is_empty(),
            "expected empty agents list, got: {agents:?}"
        );
    }

    #[tokio::test]
    async fn register_agent_tool_succeeds() {
        let server = make_server();
        let result = server
            .register_agent(Parameters(RegisterAgentArgs {
                name: "test-agent".to_owned(),
            }))
            .await
            .expect("register_agent must succeed");

        assert!(
            !result.is_error.unwrap_or(false),
            "result must not be an error"
        );
        let content = result.content.first().expect("result must have content");
        let text = content
            .as_text()
            .expect("content must be text")
            .text
            .as_str();
        let info: serde_json::Value = serde_json::from_str(text).expect("text must be valid JSON");
        assert_eq!(
            info["name"],
            serde_json::json!("test-agent"),
            "returned agent name must match"
        );
    }

    #[tokio::test]
    async fn list_agents_tool_reflects_registered_agent() {
        let server = make_server();
        server
            .register_agent(Parameters(RegisterAgentArgs {
                name: "my-agent".to_owned(),
            }))
            .await
            .expect("register must succeed");

        let result = server
            .list_agents()
            .await
            .expect("list_agents must succeed");
        let content = result.content.first().expect("result must have content");
        let text = content.as_text().expect("text content").text.as_str();
        let agents: Vec<serde_json::Value> = serde_json::from_str(text).expect("valid JSON array");

        assert_eq!(
            agents.len(),
            1,
            "should have exactly one agent after registering"
        );
        assert_eq!(
            agents[0]["name"],
            serde_json::json!("my-agent"),
            "agent name must match"
        );
    }

    #[tokio::test]
    async fn list_tools_tool_returns_empty_initially() {
        let server = make_server();
        let result = server.list_tools().await.expect("list_tools must succeed");

        assert!(!result.is_error.unwrap_or(false), "must not be an error");
        let text = result
            .content
            .first()
            .expect("content present")
            .as_text()
            .expect("text")
            .text
            .as_str();
        let tools: Vec<serde_json::Value> = serde_json::from_str(text).expect("valid JSON");
        assert!(tools.is_empty(), "expected no tools initially");
    }

    #[tokio::test]
    async fn list_conversations_tool_returns_empty_initially() {
        let server = make_server();
        let result = server
            .list_conversations()
            .await
            .expect("list_conversations must succeed");

        assert!(!result.is_error.unwrap_or(false), "must not be an error");
        let text = result
            .content
            .first()
            .expect("content present")
            .as_text()
            .expect("text")
            .text
            .as_str();
        let convs: Vec<serde_json::Value> = serde_json::from_str(text).expect("valid JSON");
        assert!(convs.is_empty(), "expected no conversations initially");
    }

    #[tokio::test]
    async fn send_message_tool_requires_valid_conversation_id() {
        let server = make_server();
        let result = server
            .send_message(Parameters(SendMessageArgs {
                conversation_id: "not-a-uuid".to_owned(),
                sender_agent_id: "00000000-0000-0000-0000-000000000001".to_owned(),
                content: "hello".to_owned(),
            }))
            .await;

        // Invalid UUID parse returns Err from McpError — either the Result is Err
        // or it's Ok with is_error set. Check for either.
        assert!(
            result.is_err() || result.unwrap().is_error.unwrap_or(false),
            "invalid conversation_id must produce an error"
        );
    }
}
