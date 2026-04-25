//! A2A service implementation backed by trumpet's core domain.
//!
//! Implements the official A2A protocol (`lf.a2a.v1.A2AService`) with
//! trumpet's agent registry, chat manager, and tool registry as the
//! backing stores.

use std::pin::Pin;
use std::sync::Arc;

use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

use crate::core::task_types::{ContextId, TaskId};
use crate::core::{Event, PushNotificationAuth, PushNotificationId, TaskFacade, TaskFilter};
use crate::grpc::convert;
use crate::grpc::proto;
use crate::server::AppState;

/// Buffer size for the per-stream channel between the broadcast subscription
/// and the gRPC client. Slow consumers cap memory at this many pending events
/// before back-pressure pushes them off the broadcast bus instead.
const STREAM_CHANNEL_CAPACITY: usize = 64;

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

    /// Borrow the shared task facade owned by [`AppState`]. Returns a cheap
    /// `Arc` clone — every transport adapter shares the same facade
    /// instance so we don't allocate a fresh router per request.
    fn facade(&self) -> Arc<TaskFacade> {
        Arc::clone(&self.state.task_facade)
    }

    /// Spawn a background task that subscribes to the message bus, filters
    /// events for `task_id`, converts them to [`proto::StreamResponse`], and
    /// forwards them on a freshly-created mpsc channel returned as a
    /// [`ReceiverStream`].
    ///
    /// If `initial_task` is `Some`, the stream begins by yielding the full
    /// task snapshot so the client never misses the initial state. The
    /// snapshot is also retained inside the spawn closure as the cached
    /// reference for artifact-update lookups; on each `TaskArtifactAdded`
    /// event we re-fetch the task via the facade to get the latest
    /// artifact body.
    fn spawn_task_stream(
        &self,
        task_id: TaskId,
        context_id: ContextId,
        initial_task: Option<crate::core::Task>,
    ) -> ReceiverStream<Result<proto::StreamResponse, Status>> {
        let (tx, rx) = tokio::sync::mpsc::channel(STREAM_CHANNEL_CAPACITY);
        let mut bus_rx = self.state.bus.subscribe();
        let facade = self.facade();

        tokio::spawn(async move {
            // Emit initial task snapshot so the client never misses state.
            if let Some(task) = initial_task.as_ref() {
                let envelope = proto::StreamResponse {
                    payload: Some(proto::stream_response::Payload::Task(
                        convert::core_task_to_proto(task),
                    )),
                };
                if tx.send(Ok(envelope)).await.is_err() {
                    return;
                }
            }

            let mut latest_task = initial_task;
            loop {
                let event = match bus_rx.recv().await {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(
                            task_id = %task_id,
                            skipped = n,
                            "stream subscriber lagged; events were dropped"
                        );
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };

                // Refresh cached task for artifact lookups when an artifact
                // event arrives; the bus carries only the artifact id.
                if matches!(&event, Event::TaskArtifactAdded { task_id: tid, .. } if *tid == task_id)
                    && let Ok(refreshed) = facade.get_task(&task_id).await
                {
                    latest_task = Some(refreshed);
                }

                if let Some(envelope) = convert::task_event_to_stream_response(
                    &event,
                    &task_id,
                    &context_id,
                    latest_task.as_ref(),
                ) && tx.send(Ok(envelope)).await.is_err()
                {
                    break;
                }
            }
        });

        ReceiverStream::new(rx)
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

        let facade = self.facade();
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
        request: Request<proto::SendMessageRequest>,
    ) -> Result<Response<Self::SendStreamingMessageStream>, Status> {
        let req = request.into_inner();
        let proto_msg = req
            .message
            .ok_or_else(|| Status::invalid_argument("message is required"))?;

        let core_msg = convert::proto_message_to_core(&proto_msg)?;

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

        let facade = self.facade();
        let task = facade
            .submit_task(core_msg, context_id, None, None)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let stream = self.spawn_task_stream(task.id, task.context_id, Some(task));
        Ok(Response::new(Box::pin(stream)))
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

        let facade = self.facade();
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

        // Cursor pagination: tasks are returned in stable id order; the
        // page_token is the id of the last item from the previous page.
        // Clients should treat it as opaque and round-trip it unchanged.
        let page_size = req.page_size.unwrap_or(50).clamp(1, 100) as usize;

        let facade = self.facade();
        let mut tasks = facade.list_tasks(&filter).await;
        tasks.sort_by_key(|t| t.id);

        let total_size = i32::try_from(tasks.len()).unwrap_or_else(|_| {
            tracing::warn!(
                count = tasks.len(),
                "task count exceeds i32::MAX; reporting i32::MAX"
            );
            i32::MAX
        });

        let start_idx = if req.page_token.is_empty() {
            0
        } else {
            tasks
                .iter()
                .position(|t| t.id.to_string() > req.page_token)
                .unwrap_or(tasks.len())
        };
        let end_idx = start_idx.saturating_add(page_size).min(tasks.len());
        let page_slice = &tasks[start_idx..end_idx];

        let next_page_token = if end_idx < tasks.len() {
            page_slice
                .last()
                .map(|t| t.id.to_string())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let proto_tasks: Vec<proto::Task> =
            page_slice.iter().map(convert::core_task_to_proto).collect();
        let returned_page_size = i32::try_from(proto_tasks.len()).unwrap_or(i32::MAX);

        Ok(Response::new(proto::ListTasksResponse {
            tasks: proto_tasks,
            next_page_token,
            page_size: returned_page_size,
            total_size,
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

        let facade = self.facade();
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
        request: Request<proto::SubscribeToTaskRequest>,
    ) -> Result<Response<Self::SubscribeToTaskStream>, Status> {
        let req = request.into_inner();
        let task_id: TaskId = req
            .id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;

        let facade = self.facade();
        let task = facade
            .get_task(&task_id)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;

        let stream = self.spawn_task_stream(task.id, task.context_id, Some(task));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn create_task_push_notification_config(
        &self,
        request: Request<proto::TaskPushNotificationConfig>,
    ) -> Result<Response<proto::TaskPushNotificationConfig>, Status> {
        let req = request.into_inner();
        let task_id: TaskId = req
            .task_id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;

        // Verify the task exists before registering a webhook against it.
        let facade = self.facade();
        facade
            .get_task(&task_id)
            .await
            .map_err(|e| Status::not_found(e.to_string()))?;

        let auth = req.authentication.as_ref().map(|a| PushNotificationAuth {
            scheme: a.scheme.clone(),
            credentials: a.credentials.clone(),
        });

        let mut store = self.state.push_notifications.write().await;
        let cfg = store
            .create(task_id, req.url.clone(), req.token.clone(), auth)
            .map_err(|e| match e {
                crate::error::Error::InvalidInput { reason } => Status::invalid_argument(reason),
                other => Status::internal(other.to_string()),
            })?;

        Ok(Response::new(push_config_to_proto(&cfg)))
    }

    async fn get_task_push_notification_config(
        &self,
        request: Request<proto::GetTaskPushNotificationConfigRequest>,
    ) -> Result<Response<proto::TaskPushNotificationConfig>, Status> {
        let req = request.into_inner();
        let task_id: TaskId = req
            .task_id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;
        let cfg_id: PushNotificationId = req
            .id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid push notification config id"))?;

        let store = self.state.push_notifications.read().await;
        let cfg = store
            .get(&task_id, &cfg_id)
            .ok_or_else(|| Status::not_found("push notification config not found"))?;

        Ok(Response::new(push_config_to_proto(cfg)))
    }

    async fn list_task_push_notification_configs(
        &self,
        request: Request<proto::ListTaskPushNotificationConfigsRequest>,
    ) -> Result<Response<proto::ListTaskPushNotificationConfigsResponse>, Status> {
        let req = request.into_inner();
        let task_id: TaskId = req
            .task_id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;

        let store = self.state.push_notifications.read().await;
        let configs: Vec<proto::TaskPushNotificationConfig> = store
            .list(&task_id)
            .iter()
            .map(push_config_to_proto)
            .collect();

        Ok(Response::new(
            proto::ListTaskPushNotificationConfigsResponse {
                configs,
                // No pagination yet — push-notification config lists are typically
                // very small per task.
                next_page_token: String::new(),
            },
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
                streaming: Some(true),
                push_notifications: Some(true),
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
        request: Request<proto::DeleteTaskPushNotificationConfigRequest>,
    ) -> Result<Response<()>, Status> {
        let req = request.into_inner();
        let task_id: TaskId = req
            .task_id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid task id"))?;
        let cfg_id: PushNotificationId = req
            .id
            .parse()
            .map_err(|_| Status::invalid_argument("invalid push notification config id"))?;

        let mut store = self.state.push_notifications.write().await;
        if store.delete(&task_id, &cfg_id) {
            Ok(Response::new(()))
        } else {
            Err(Status::not_found("push notification config not found"))
        }
    }
}

/// Convert a core [`crate::core::PushNotificationConfig`] to the proto
/// wire type used by the four push-notification RPCs.
fn push_config_to_proto(
    cfg: &crate::core::PushNotificationConfig,
) -> proto::TaskPushNotificationConfig {
    proto::TaskPushNotificationConfig {
        tenant: String::new(),
        id: cfg.id.to_string(),
        task_id: cfg.task_id.to_string(),
        url: cfg.url.clone(),
        token: cfg.token.clone(),
        authentication: cfg
            .authentication
            .as_ref()
            .map(|a| proto::AuthenticationInfo {
                scheme: a.scheme.clone(),
                credentials: a.credentials.clone(),
            }),
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

    // ── pagination ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_tasks_paginates_with_cursor() {
        let svc = make_service();

        // Create 5 tasks via send_message.
        for i in 0..5 {
            let msg = proto::Message {
                message_id: uuid::Uuid::new_v4().to_string(),
                role: proto::Role::User.into(),
                parts: vec![proto::Part {
                    content: Some(proto::part::Content::Text(format!("msg{i}"))),
                    ..Default::default()
                }],
                ..Default::default()
            };
            proto::a2a_service_server::A2aService::send_message(
                &svc,
                Request::new(proto::SendMessageRequest {
                    message: Some(msg),
                    ..Default::default()
                }),
            )
            .await
            .expect("send must succeed");
        }

        // First page: size 2.
        let page1 = proto::a2a_service_server::A2aService::list_tasks(
            &svc,
            Request::new(proto::ListTasksRequest {
                page_size: Some(2),
                ..Default::default()
            }),
        )
        .await
        .expect("list must succeed")
        .into_inner();

        assert_eq!(page1.tasks.len(), 2, "first page must contain 2 items");
        assert_eq!(page1.page_size, 2, "page_size must reflect actual count");
        assert_eq!(page1.total_size, 5, "total_size must reflect total count");
        assert!(
            !page1.next_page_token.is_empty(),
            "next_page_token must be non-empty when more pages exist"
        );

        // Second page using the cursor.
        let page2 = proto::a2a_service_server::A2aService::list_tasks(
            &svc,
            Request::new(proto::ListTasksRequest {
                page_size: Some(2),
                page_token: page1.next_page_token.clone(),
                ..Default::default()
            }),
        )
        .await
        .expect("list page 2 must succeed")
        .into_inner();

        assert_eq!(page2.tasks.len(), 2, "second page must contain 2 items");
        assert!(
            !page2.next_page_token.is_empty(),
            "third page should still exist (1 item left)"
        );
        // Different items than page 1.
        let page1_ids: std::collections::HashSet<_> =
            page1.tasks.iter().map(|t| t.id.clone()).collect();
        for t in &page2.tasks {
            assert!(
                !page1_ids.contains(&t.id),
                "page 2 must not duplicate page 1 items"
            );
        }

        // Third page: 1 remaining item, no further token.
        let page3 = proto::a2a_service_server::A2aService::list_tasks(
            &svc,
            Request::new(proto::ListTasksRequest {
                page_size: Some(2),
                page_token: page2.next_page_token.clone(),
                ..Default::default()
            }),
        )
        .await
        .expect("list page 3 must succeed")
        .into_inner();

        assert_eq!(page3.tasks.len(), 1, "third page must contain 1 item");
        assert!(
            page3.next_page_token.is_empty(),
            "next_page_token must be empty on the last page"
        );
    }

    // ── push notifications ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn push_notification_create_and_list_round_trip() {
        let svc = make_service();

        // Create a task first.
        let send_resp = proto::a2a_service_server::A2aService::send_message(
            &svc,
            Request::new(proto::SendMessageRequest {
                message: Some(text_message()),
                ..Default::default()
            }),
        )
        .await
        .expect("send must succeed");
        let task_id = match send_resp.into_inner().payload {
            Some(proto::send_message_response::Payload::Task(t)) => t.id,
            other => panic!("expected Task, got: {other:?}"),
        };

        // Register two webhooks.
        for url in ["https://hook-a.example/", "https://hook-b.example/"] {
            proto::a2a_service_server::A2aService::create_task_push_notification_config(
                &svc,
                Request::new(proto::TaskPushNotificationConfig {
                    task_id: task_id.clone(),
                    url: url.to_owned(),
                    ..Default::default()
                }),
            )
            .await
            .expect("create push config must succeed");
        }

        // List.
        let listed = proto::a2a_service_server::A2aService::list_task_push_notification_configs(
            &svc,
            Request::new(proto::ListTaskPushNotificationConfigsRequest {
                task_id: task_id.clone(),
                ..Default::default()
            }),
        )
        .await
        .expect("list must succeed")
        .into_inner();
        assert_eq!(listed.configs.len(), 2);
    }

    #[tokio::test]
    async fn push_notification_create_rejects_invalid_url() {
        let svc = make_service();

        // Create a task to attach to.
        let send_resp = proto::a2a_service_server::A2aService::send_message(
            &svc,
            Request::new(proto::SendMessageRequest {
                message: Some(text_message()),
                ..Default::default()
            }),
        )
        .await
        .expect("send must succeed");
        let task_id = match send_resp.into_inner().payload {
            Some(proto::send_message_response::Payload::Task(t)) => t.id,
            other => panic!("expected Task, got: {other:?}"),
        };

        let result = proto::a2a_service_server::A2aService::create_task_push_notification_config(
            &svc,
            Request::new(proto::TaskPushNotificationConfig {
                task_id,
                url: "not-a-valid-url".to_owned(),
                ..Default::default()
            }),
        )
        .await;
        let err = result.expect_err("invalid url must be rejected");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }

    #[tokio::test]
    async fn push_notification_get_unknown_returns_not_found() {
        let svc = make_service();
        let result = proto::a2a_service_server::A2aService::get_task_push_notification_config(
            &svc,
            Request::new(proto::GetTaskPushNotificationConfigRequest {
                task_id: uuid::Uuid::new_v4().to_string(),
                id: uuid::Uuid::new_v4().to_string(),
                ..Default::default()
            }),
        )
        .await;
        let err = result.expect_err("unknown config must return error");
        assert_eq!(err.code(), tonic::Code::NotFound);
    }

    // ── streaming ──────────────────────────────────────────────────────────────

    fn text_message() -> proto::Message {
        proto::Message {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: proto::Role::User.into(),
            parts: vec![proto::Part {
                content: Some(proto::part::Content::Text("hello".to_owned())),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn send_streaming_message_yields_initial_task() {
        use tokio_stream::StreamExt as _;

        let svc = make_service();
        let req = Request::new(proto::SendMessageRequest {
            message: Some(text_message()),
            ..Default::default()
        });
        let resp = proto::a2a_service_server::A2aService::send_streaming_message(&svc, req)
            .await
            .expect("streaming send must start");

        let mut stream = resp.into_inner();
        let first = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next())
            .await
            .expect("initial event must arrive within 200ms")
            .expect("stream must yield at least one item")
            .expect("first item must be Ok");

        assert!(matches!(
            first.payload,
            Some(proto::stream_response::Payload::Task(_))
        ));
    }

    #[tokio::test]
    async fn subscribe_to_unknown_task_returns_not_found() {
        let svc = make_service();
        let req = Request::new(proto::SubscribeToTaskRequest {
            id: uuid::Uuid::new_v4().to_string(),
            ..Default::default()
        });
        let result = proto::a2a_service_server::A2aService::subscribe_to_task(&svc, req).await;
        // Stream response type is not Debug, so use a manual match instead of
        // unwrap_err.
        match result {
            Err(status) => assert_eq!(status.code(), tonic::Code::NotFound),
            Ok(_) => panic!("expected NotFound, got Ok"),
        }
    }

    #[tokio::test]
    async fn subscribe_yields_status_update_when_task_progresses() {
        use tokio_stream::StreamExt as _;

        let svc = make_service();

        // Create a task first.
        let send = proto::a2a_service_server::A2aService::send_message(
            &svc,
            Request::new(proto::SendMessageRequest {
                message: Some(text_message()),
                ..Default::default()
            }),
        )
        .await
        .expect("send_message must succeed");

        let task_id = match send.into_inner().payload {
            Some(proto::send_message_response::Payload::Task(t)) => t.id,
            other => panic!("expected Task payload, got: {other:?}"),
        };

        // Subscribe.
        let resp = proto::a2a_service_server::A2aService::subscribe_to_task(
            &svc,
            Request::new(proto::SubscribeToTaskRequest {
                id: task_id.clone(),
                ..Default::default()
            }),
        )
        .await
        .expect("subscribe must succeed");

        let mut stream = resp.into_inner();

        // Drain the initial task snapshot.
        let _initial = tokio::time::timeout(std::time::Duration::from_millis(200), stream.next())
            .await
            .expect("initial event must arrive")
            .expect("stream must yield")
            .expect("first item must be Ok");

        // Trigger a state change via the facade.
        let facade = svc.facade();
        let parsed: TaskId = task_id.parse().unwrap();
        facade
            .update_status(&parsed, crate::core::TaskState::Working, None)
            .await
            .expect("state update must succeed");

        let next = tokio::time::timeout(std::time::Duration::from_millis(500), stream.next())
            .await
            .expect("status update must arrive within 500ms")
            .expect("stream must yield")
            .expect("event must be Ok");

        match next.payload {
            Some(proto::stream_response::Payload::StatusUpdate(update)) => {
                assert_eq!(update.task_id, task_id);
                assert_eq!(
                    update.status.expect("status").state,
                    proto::TaskState::Working as i32
                );
            }
            other => panic!("expected StatusUpdate, got: {other:?}"),
        }
    }
}
