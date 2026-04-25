# ADR-012: Task System — Lifecycle & Storage

## Status

Accepted

## Context

Trumpet orchestrates AI agents but currently lacks a task concept. The official A2A protocol (`vendor/a2a/specification/a2a.proto`) is task-centric: agents communicate by submitting tasks that progress through defined lifecycle states. Without tasks as first-class entities, trumpet cannot implement A2A, cannot route work between agents, and cannot track the progress of multi-step operations across protocol boundaries.

## Decision

We will implement tasks as core domain entities stored in a `TaskManager` (following the same pattern as `ChatManager`). Tasks are in-memory, indexed by `TaskId` and grouped by `ContextId`, with persistence via the existing `StateSnapshot`.

### State Machine

Tasks follow the A2A state machine:

```text
                    +---> REJECTED (terminal)
                    |
SUBMITTED ---+----> WORKING ---+---> COMPLETED (terminal)
                    ^    |     +---> FAILED (terminal)
                    |    |     +---> CANCELED (terminal)
                    |    +---> INPUT_REQUIRED --+
                    |    +---> AUTH_REQUIRED ----+
                    |                            |
                    +----------------------------+
```

- **Terminal states**: Completed, Failed, Canceled, Rejected
- **Interrupted states**: InputRequired, AuthRequired (can resume to Working)
- Transitions are validated — invalid transitions return an error
- Every transition publishes a `TaskEvent` on the message bus

### Core Types

New file `src/core/task_types.rs`:

- `TaskId(Uuid)`, `ContextId(Uuid)`, `ArtifactId(Uuid)` — UUID newtypes
- `TaskState` enum with `is_terminal()` and `can_transition_to()` methods
- `Task { id, context_id, status, artifacts, history, metadata, assignee, creator }`
- `TaskMessage { id, role, parts, metadata }` — the A2A Message concept
- `Part` enum: Text, Raw, Url, Data — with optional filename and media_type
- `Artifact { id, name, description, parts, metadata }`
- `TaskFilter` for querying by context, state, assignee

### Persistence

The `StateSnapshot` gains a `tasks: Vec<Task>` field with `#[serde(default)]` for backward compatibility with existing snapshots.

## Consequences

- Tasks unify work across protocols — MCP tool calls, gRPC SendMessage, and REST POST all create the same core Task.
- The state machine prevents invalid transitions and serves as a contract between task creators and handlers.
- Bus events enable real-time streaming across all transports without polling.
- Adds complexity to the core domain but is essential for the nexus model.
- Backward compatible with existing snapshots.
