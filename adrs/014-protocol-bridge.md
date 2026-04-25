# ADR-014: Protocol Bridge — Unified Task Facade

## Status

Accepted

## Context

Trumpet exposes multiple protocol interfaces (MCP, gRPC/A2A, REST, WebSocket). All need to perform the same task operations: submit, update status, add artifacts, cancel, query. Without a unified entry point, each transport adapter implements its own task logic, leading to inconsistency, duplication, and divergent behavior across protocols.

## Decision

A `TaskFacade` in `src/core/task_facade.rs` provides the canonical task API. Every protocol adapter calls through this facade. Protocol-specific type conversion happens at the adapter boundary, not in core.

### Facade API

```rust
impl TaskFacade {
    pub async fn submit_task(&self, message, context_id, assignee, metadata) -> Result<Task>;
    pub async fn update_status(&self, task_id, new_state, message) -> Result<Task>;
    pub async fn add_artifact(&self, task_id, artifact) -> Result<Task>;
    pub async fn cancel_task(&self, task_id) -> Result<Task>;
    pub async fn get_task(&self, task_id) -> Result<Task>;
    pub async fn list_tasks(&self, filter) -> Vec<Task>;
    pub async fn subscribe(&self, task_id) -> Receiver<TaskEvent>;
}
```

The facade orchestrates: validation → state transition → routing → bus events.

### Transport Adapters

Each adapter is thin — type conversion + facade calls, no business logic:

- **gRPC** (`src/grpc/service.rs`): proto ↔ core types via `src/grpc/convert.rs`
- **MCP** (`src/mcp/handler.rs`): adds `submit_task`, `get_task`, `list_tasks`, `cancel_task` tools
- **REST** (`src/server/routes.rs`): `POST /tasks`, `GET /tasks/{id}`, `GET /tasks`, `POST /tasks/{id}/cancel`
- **WebSocket/SSE**: task events flow through the existing bus → already handled

### Invocation Flow

```text
Client (MCP / gRPC / REST)
  ↓ protocol-specific request
Transport Adapter (thin)
  ↓ convert to core types
TaskFacade
  ↓ validate → route → transition → publish event
TaskManager + MessageBus
  ↓ event propagated
All subscribers (SSE, WebSocket, gRPC streams, MCP notifications)
```

## Consequences

- Single source of truth for task logic. Changes to validation, state transitions, or routing happen in one place.
- Transport adapters stay small and focused on protocol marshalling.
- Adding a new protocol requires only type conversion + facade calls.
- The facade is a critical path — must be well-tested.
