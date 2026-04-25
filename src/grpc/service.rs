//! A2A service implementation backed by trumpet's core domain.
//!
//! Implements the official A2A protocol (`lf.a2a.v1.A2AService`) with
//! trumpet's agent registry, chat manager, and tool registry as the
//! backing stores.

use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::core::{DefaultTaskRouter, TaskFacade, TaskFilter};
use crate::grpc::convert;
use crate::grpc::proto;
use crate::server::AppState;

/// Trumpet's implementation of the A2A protocol service.
#[derive(Clone)]
pub struct NexusA2aService {
    state: AppState,
}

impl NexusA2aService {
    /// Create a new service backed by the given shared state.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    fn make_facade(&self) -> TaskFacade {
        TaskFacade::new(
            Arc::clone(&self.state.tasks),
            Arc::clone(&self.state.registry),
            Box::new(DefaultTaskRouter),
        )
    }
}

type StreamResult<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl proto::a2a_service_server::A2aService for NexusA2aService {
    async fn send_message(
        &self,
        request: Request<proto::SendMessageRequest>,
    ) -> Result<Response<proto::SendMessageResponse>, Status> {
        let req = request.into_inner();
        let proto_msg = req
            .message
            .ok_or_else(|| Status::invalid_argument("message is required"))?;

        let core_msg = convert::proto_message_to_core(&proto_msg)?;

        // Extract context_id if provided on the message.
        let context_id = if proto_msg.context_id.is_empty() {
            None
        } else {
            Some(
                proto_msg
                    .context_id
                    .parse()
                    .map_err(|_| Status::invalid_argument("invalid context_id"))?,
            )
        };

        // Extract assignee from task_id if this is a reply to an existing task.
        // For new tasks, we let the router decide.
        let facade = self.make_facade();
        let task = facade
            .submit_task(core_msg, context_id, None, None)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let proto_task = convert::core_task_to_proto(&task);
        Ok(Response::new(proto::SendMessageResponse {
            payload: Some(proto::send_message_response::Payload::Task(proto_task)),
        }))
    }

    type SendStreamingMessageStream = StreamResult<proto::StreamResponse>;

    async fn send_streaming_message(
        &self,
        _request: Request<proto::SendMessageRequest>,
    ) -> Result<Response<Self::SendStreamingMessageStream>, Status> {
        Err(Status::unimplemented(
            "send_streaming_message not yet implemented",
        ))
    }

    async fn get_task(
        &self,
        request: Request<proto::GetTaskRequest>,
    ) -> Result<Response<proto::Task>, Status> {
        let req = request.into_inner();
        let task_id = req
            .id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;

        let facade = self.make_facade();
        let task = facade
            .get_task(&task_id)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;

        Ok(Response::new(convert::core_task_to_proto(&task)))
    }

    async fn list_tasks(
        &self,
        request: Request<proto::ListTasksRequest>,
    ) -> Result<Response<proto::ListTasksResponse>, Status> {
        let req = request.into_inner();

        let context_id = if req.context_id.is_empty() {
            None
        } else {
            Some(
                req.context_id
                    .parse()
                    .map_err(|_| Status::invalid_argument("invalid context_id"))?,
            )
        };

        let state_filter = if req.status == 0 {
            None
        } else {
            Some(convert::proto_state_to_core(req.status)?)
        };

        let filter = TaskFilter {
            context_id,
            state: state_filter,
            assignee: None,
        };

        let facade = self.make_facade();
        let tasks = facade.list_tasks(&filter).await;
        let proto_tasks: Vec<proto::Task> = tasks.iter().map(convert::core_task_to_proto).collect();
        let total = proto_tasks.len() as i32;

        Ok(Response::new(proto::ListTasksResponse {
            tasks: proto_tasks,
            next_page_token: String::new(),
            page_size: total,
            total_size: total,
        }))
    }

    async fn cancel_task(
        &self,
        request: Request<proto::CancelTaskRequest>,
    ) -> Result<Response<proto::Task>, Status> {
        let req = request.into_inner();
        let task_id = req
            .id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;

        let facade = self.make_facade();
        let task = facade
            .cancel_task(&task_id, None)
            .await
            .map_err(|e| match e {
                crate::error::Error::TaskNotFound { .. } => Status::not_found(e.to_string()),
                crate::error::Error::TaskAlreadyTerminal { .. } => {
                    Status::failed_precondition(e.to_string())
                }
                _ => Status::internal(e.to_string()),
            })?;

        Ok(Response::new(convert::core_task_to_proto(&task)))
    }

    type SubscribeToTaskStream = StreamResult<proto::StreamResponse>;

    async fn subscribe_to_task(
        &self,
        _request: Request<proto::SubscribeToTaskRequest>,
    ) -> Result<Response<Self::SubscribeToTaskStream>, Status> {
        Err(Status::unimplemented(
            "subscribe_to_task not yet implemented",
        ))
    }

    async fn create_task_push_notification_config(
        &self,
        _request: Request<proto::TaskPushNotificationConfig>,
    ) -> Result<Response<proto::TaskPushNotificationConfig>, Status> {
        Err(Status::unimplemented(
            "create_task_push_notification_config not yet implemented",
        ))
    }

    async fn get_task_push_notification_config(
        &self,
        _request: Request<proto::GetTaskPushNotificationConfigRequest>,
    ) -> Result<Response<proto::TaskPushNotificationConfig>, Status> {
        Err(Status::unimplemented(
            "get_task_push_notification_config not yet implemented",
        ))
    }

    async fn list_task_push_notification_configs(
        &self,
        _request: Request<proto::ListTaskPushNotificationConfigsRequest>,
    ) -> Result<Response<proto::ListTaskPushNotificationConfigsResponse>, Status> {
        Err(Status::unimplemented(
            "list_task_push_notification_configs not yet implemented",
        ))
    }

    async fn get_extended_agent_card(
        &self,
        _request: Request<proto::GetExtendedAgentCardRequest>,
    ) -> Result<Response<proto::AgentCard>, Status> {
        let config = &self.state.config;
        let tools = self.state.tools.read().await;

        let skills: Vec<proto::AgentSkill> = tools
            .list()
            .iter()
            .map(|t| proto::AgentSkill {
                id: t.id.to_string(),
                name: t.name.clone(),
                description: t.description.clone(),
                tags: vec![],
                examples: vec![],
                input_modes: vec![],
                output_modes: vec![],
                security_requirements: vec![],
            })
            .collect();

        let card = proto::AgentCard {
            name: "trumpet".to_owned(),
            description: "Trumpet agent nexus daemon".to_owned(),
            supported_interfaces: vec![proto::AgentInterface {
                url: format!("grpc://{}:{}", config.server.host, config.server.grpc_port),
                protocol_binding: "GRPC".to_owned(),
                tenant: String::new(),
                protocol_version: "0.3".to_owned(),
            }],
            provider: Some(proto::AgentProvider {
                url: String::new(),
                organization: "trumpet".to_owned(),
            }),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            documentation_url: None,
            capabilities: Some(proto::AgentCapabilities {
                streaming: Some(false),
                push_notifications: Some(false),
                extensions: vec![],
                extended_agent_card: Some(true),
            }),
            security_schemes: Default::default(),
            security_requirements: vec![],
            default_input_modes: vec!["text/plain".to_owned()],
            default_output_modes: vec!["text/plain".to_owned()],
            skills,
            signatures: vec![],
            icon_url: None,
        };

        Ok(Response::new(card))
    }

    async fn delete_task_push_notification_config(
        &self,
        _request: Request<proto::DeleteTaskPushNotificationConfigRequest>,
    ) -> Result<Response<()>, Status> {
        Err(Status::unimplemented(
            "delete_task_push_notification_config not yet implemented",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn make_service() -> NexusA2aService {
        let state = AppState::new(Config::default());
        NexusA2aService::new(state)
    }

    #[tokio::test]
    async fn send_message_creates_task() {
        let svc = make_service();
        let msg = proto::Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: proto::Role::User.into(),
            parts: vec![proto::Part {
                content: Some(proto::part::Content::Text("hello".to_owned())),
                ..Default::default()
            }],
            ..Default::default()
        };
        let req = Request::new(proto::SendMessageRequest {
            message: Some(msg),
            ..Default::default()
        });
        let resp = proto::a2a_service_server::A2aService::send_message(&svc, req)
            .await
            .expect("send_message must succeed");
        let inner = resp.into_inner();
        assert!(
            matches!(
                inner.payload,
                Some(proto::send_message_response::Payload::Task(_))
            ),
            "response must contain a task"
        );
    }

    #[tokio::test]
    async fn get_task_not_found() {
        let svc = make_service();
        let req = Request::new(proto::GetTaskRequest {
            id: uuid::Uuid::new_v4().to_string(),
            ..Default::default()
        });
        let result = proto::a2a_service_server::A2aService::get_task(&svc, req).await;
        assert!(result.is_err(), "unknown task must return error");
        assert_eq!(result.unwrap_err().code(), tonic::Code::NotFound);
    }

    #[tokio::test]
    async fn list_tasks_returns_empty_initially() {
        let svc = make_service();
        let req = Request::new(proto::ListTasksRequest::default());
        let resp = proto::a2a_service_server::A2aService::list_tasks(&svc, req)
            .await
            .expect("list_tasks must succeed");
        assert!(
            resp.into_inner().tasks.is_empty(),
            "no tasks should exist initially"
        );
    }

    #[tokio::test]
    async fn get_extended_agent_card_returns_card() {
        let svc = make_service();
        let req = Request::new(proto::GetExtendedAgentCardRequest::default());
        let resp = proto::a2a_service_server::A2aService::get_extended_agent_card(&svc, req)
            .await
            .expect("get_extended_agent_card must succeed");
        let card = resp.into_inner();
        assert_eq!(card.name, "trumpet");
        assert!(!card.version.is_empty());
    }

    #[tokio::test]
    async fn send_message_without_message_field_returns_error() {
        let svc = make_service();
        let req = Request::new(proto::SendMessageRequest::default());
        let result = proto::a2a_service_server::A2aService::send_message(&svc, req).await;
        assert!(result.is_err(), "missing message must return error");
        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }
}
