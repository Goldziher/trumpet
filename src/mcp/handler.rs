//! MCP server handler for the Trumpet agent nexus.
//!
//! [`TrumpetMcpServer`] exposes tools for managing agents, skills, and
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
                .register(&args.name)
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

    /// List all registered skills.
    #[tool(description = "List all skills registered with the Trumpet nexus.")]
    async fn list_skills(&self) -> Result<CallToolResult, McpError> {
        let skills = self.state.skills.read().await;
        let skill_list: Vec<_> = skills.list().into_iter().cloned().collect();
        let json = serde_json::to_string(&skill_list)
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
}

// ── ServerHandler ─────────────────────────────────────────────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TrumpetMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("trumpet", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions("Trumpet agent nexus — manage agents, skills, and conversations.")
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
    async fn list_skills_tool_returns_empty_initially() {
        let server = make_server();
        let result = server
            .list_skills()
            .await
            .expect("list_skills must succeed");

        assert!(!result.is_error.unwrap_or(false), "must not be an error");
        let text = result
            .content
            .first()
            .expect("content present")
            .as_text()
            .expect("text")
            .text
            .as_str();
        let skills: Vec<serde_json::Value> = serde_json::from_str(text).expect("valid JSON");
        assert!(skills.is_empty(), "expected no skills initially");
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
