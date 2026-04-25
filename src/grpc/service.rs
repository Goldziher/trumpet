//! A2A service implementation backed by trumpet's core domain.
//!
//! Implements the official A2A protocol (`lf.a2a.v1.A2AService`) with
//! trumpet's agent registry, chat manager, and skill registry as the
//! backing stores.

use std::pin::Pin;

use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::grpc::proto;
use crate::server::AppState;

/// Trumpet's implementation of the A2A protocol service.
#[derive(Clone)]
pub struct NexusA2aService {
    #[expect(dead_code, reason = "used when RPC methods are implemented")]
    state: AppState,
}

impl NexusA2aService {
    /// Create a new service backed by the given shared state.
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

type StreamResult<T> = Pin<Box<dyn Stream<Item = Result<T, Status>> + Send + 'static>>;

#[tonic::async_trait]
impl proto::a2a_service_server::A2aService for NexusA2aService {
    async fn send_message(
        &self,
        _request: Request<proto::SendMessageRequest>,
    ) -> Result<Response<proto::SendMessageResponse>, Status> {
        Err(Status::unimplemented("send_message not yet implemented"))
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
        _request: Request<proto::GetTaskRequest>,
    ) -> Result<Response<proto::Task>, Status> {
        Err(Status::unimplemented("get_task not yet implemented"))
    }

    async fn list_tasks(
        &self,
        _request: Request<proto::ListTasksRequest>,
    ) -> Result<Response<proto::ListTasksResponse>, Status> {
        Err(Status::unimplemented("list_tasks not yet implemented"))
    }

    async fn cancel_task(
        &self,
        _request: Request<proto::CancelTaskRequest>,
    ) -> Result<Response<proto::Task>, Status> {
        Err(Status::unimplemented("cancel_task not yet implemented"))
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
        Err(Status::unimplemented(
            "get_extended_agent_card not yet implemented",
        ))
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
    async fn send_message_returns_unimplemented() {
        let svc = make_service();
        let req = Request::new(proto::SendMessageRequest::default());
        let result = proto::a2a_service_server::A2aService::send_message(&svc, req).await;
        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }

    #[tokio::test]
    async fn get_task_returns_unimplemented() {
        let svc = make_service();
        let req = Request::new(proto::GetTaskRequest::default());
        let result = proto::a2a_service_server::A2aService::get_task(&svc, req).await;
        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
    }
}
