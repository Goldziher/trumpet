//! Tool invocation dispatcher.
//!
//! [`ToolInvoker`] resolves a tool by name from the [`ToolRegistry`] and
//! dispatches to the appropriate provider: built-in tools execute in-process,
//! agent-provided tools create tasks via the [`TaskFacade`].

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::core::code_tools::CodeTools;
use crate::core::task_types::{MessageRole, Part, TaskMessage};
use crate::core::tools::{ToolRegistry, ToolResult};
use crate::core::types::{MessageId, ToolProvider};
use crate::core::{AgentRegistry, TaskFacade};
use crate::error::Error;

/// Dispatches tool invocations to the appropriate provider.
pub struct ToolInvoker {
    tools: Arc<RwLock<ToolRegistry>>,
    code_tools: Arc<RwLock<Option<CodeTools>>>,
    facade: Arc<TaskFacade>,
    registry: Arc<RwLock<AgentRegistry>>,
}

impl ToolInvoker {
    /// Create a new [`ToolInvoker`] from shared state components.
    ///
    /// `facade` should be the daemon-wide [`TaskFacade`] kept in
    /// [`AppState`](crate::server::AppState); per-call construction was
    /// replaced with this shared instance to avoid allocating a fresh
    /// [`Box<DefaultTaskRouter>`](crate::core::DefaultTaskRouter) per
    /// agent-tool invocation.
    pub fn new(
        tools: Arc<RwLock<ToolRegistry>>,
        code_tools: Arc<RwLock<Option<CodeTools>>>,
        facade: Arc<TaskFacade>,
        registry: Arc<RwLock<AgentRegistry>>,
    ) -> Self {
        Self {
            tools,
            code_tools,
            facade,
            registry,
        }
    }

    /// Invoke a tool by name with the given JSON input.
    ///
    /// Built-in tools return [`ToolResult::Immediate`]. Agent-provided tools
    /// create a task and return [`ToolResult::TaskCreated`].
    pub async fn invoke(&self, name: &str, input: serde_json::Value) -> Result<ToolResult, Error> {
        let provider = {
            let tools = self.tools.read().await;
            let info = tools
                .find_by_name(name)
                .ok_or_else(|| Error::ToolNotFound {
                    name: name.to_owned(),
                })?;
            info.provider.clone()
        };

        match provider {
            ToolProvider::BuiltIn => {
                let code_tools = self.code_tools.read().await;
                let ct = code_tools
                    .as_ref()
                    .ok_or_else(|| Error::ToolInvocationFailed {
                        name: name.to_owned(),
                        reason: "built-in tools not initialized".to_owned(),
                    })?;
                let output = ct.dispatch(name, input).await?;
                Ok(ToolResult::Immediate { output })
            }
            ToolProvider::Agent { agent_id } => {
                // Verify agent is registered.
                {
                    let reg = self.registry.read().await;
                    let agent =
                        reg.get(&agent_id)
                            .ok_or_else(|| Error::ToolProviderUnavailable {
                                name: name.to_owned(),
                            })?;
                    if agent.status != crate::core::types::AgentStatus::Connected {
                        return Err(Error::ToolProviderUnavailable {
                            name: name.to_owned(),
                        });
                    }
                }

                let message = TaskMessage {
                    id: MessageId::new(),
                    role: MessageRole::User,
                    parts: vec![Part::Text {
                        text: serde_json::to_string(&input).map_err(|e| {
                            Error::ToolInvocationFailed {
                                name: name.to_owned(),
                                reason: format!("failed to serialize input: {e}"),
                            }
                        })?,
                    }],
                    metadata: Some(serde_json::json!({"tool_name": name})),
                };

                let task = self
                    .facade
                    .submit_task(message, None, Some(agent_id), None)
                    .await?;
                Ok(ToolResult::TaskCreated {
                    task: Box::new(task),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CodeToolsConfig;
    use crate::core::bus::MessageBus;
    use crate::core::code_tools::CodeTools;
    use crate::core::tools::ToolRegistry;

    fn lib_rs_path() -> String {
        "src/lib.rs".to_owned()
    }

    fn make_invoker() -> ToolInvoker {
        use crate::core::DefaultTaskRouter;

        let bus = Arc::new(MessageBus::new(64));
        let tools = Arc::new(RwLock::new(ToolRegistry::new(Arc::clone(&bus))));
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let code_tools = Arc::new(RwLock::new(Some(
            CodeTools::new(CodeToolsConfig {
                workspace_root: Some(manifest),
                ..CodeToolsConfig::default()
            })
            .expect("CodeTools::new must succeed for the manifest dir"),
        )));
        let tasks = Arc::new(RwLock::new(crate::core::TaskManager::new(Arc::clone(&bus))));
        let registry = Arc::new(RwLock::new(AgentRegistry::new(Arc::clone(&bus))));
        let facade = Arc::new(TaskFacade::new(
            Arc::clone(&tasks),
            Arc::clone(&registry),
            Box::new(DefaultTaskRouter),
        ));
        ToolInvoker::new(tools, code_tools, facade, registry)
    }

    #[tokio::test]
    async fn invoke_builtin_tool_returns_immediate() {
        let invoker = make_invoker();

        // Register a built-in tool.
        {
            let mut tools = invoker.tools.write().await;
            tools
                .register(
                    "code.read_file",
                    "Read a file",
                    serde_json::json!({}),
                    serde_json::json!({}),
                    ToolProvider::BuiltIn,
                )
                .unwrap();
        }

        let input = serde_json::json!({"path": lib_rs_path()});
        let result = invoker
            .invoke("code.read_file", input)
            .await
            .expect("invoke must succeed for built-in tool");

        assert!(
            matches!(result, ToolResult::Immediate { .. }),
            "built-in tool must return Immediate result"
        );
    }

    #[tokio::test]
    async fn invoke_unknown_tool_returns_not_found() {
        let invoker = make_invoker();
        let result = invoker.invoke("nonexistent", serde_json::json!({})).await;
        assert!(
            matches!(result, Err(Error::ToolNotFound { .. })),
            "unknown tool must return ToolNotFound"
        );
    }

    #[tokio::test]
    async fn invoke_agent_tool_creates_task() {
        let invoker = make_invoker();

        // Register an agent and an agent-provided tool.
        let agent_id = {
            let mut reg = invoker.registry.write().await;
            let info = reg.register("test-agent", None).unwrap();
            info.id
        };

        {
            let mut tools = invoker.tools.write().await;
            tools
                .register(
                    "agent.research",
                    "Research a topic",
                    serde_json::json!({}),
                    serde_json::json!({}),
                    ToolProvider::Agent { agent_id },
                )
                .unwrap();
        }

        let result = invoker
            .invoke("agent.research", serde_json::json!({"query": "test"}))
            .await
            .expect("invoke must succeed for agent tool");

        assert!(
            matches!(result, ToolResult::TaskCreated { .. }),
            "agent tool must return TaskCreated result"
        );
    }

    #[tokio::test]
    async fn invoke_agent_tool_disconnected_returns_unavailable() {
        let invoker = make_invoker();

        // Register an agent, then disconnect it.
        let agent_id = {
            let mut reg = invoker.registry.write().await;
            let info = reg.register("offline-agent", None).unwrap();
            info.id
        };
        {
            let mut reg = invoker.registry.write().await;
            reg.deregister(&agent_id).unwrap();
        }

        // Tool still references the agent but agent is gone.
        {
            let mut tools = invoker.tools.write().await;
            tools
                .register(
                    "agent.gone",
                    "Gone agent",
                    serde_json::json!({}),
                    serde_json::json!({}),
                    ToolProvider::Agent { agent_id },
                )
                .unwrap();
        }

        let result = invoker.invoke("agent.gone", serde_json::json!({})).await;
        assert!(
            matches!(result, Err(Error::ToolProviderUnavailable { .. })),
            "deregistered agent must return ToolProviderUnavailable"
        );
    }
}
