# ADR-013: Task Routing & Agent Selection

## Status

Accepted

## Context

When a task is submitted, trumpet must decide which agent handles it. The A2A protocol does not prescribe routing — that is the nexus's responsibility. Routing must work across protocol boundaries: an MCP-connected agent can receive work originally submitted via gRPC. The architecture document (`architecture.md`) already references a Task Router at `core/router.rs` that was never implemented.

## Decision

We will implement task routing as a trait with three strategies in priority order:

1. **Explicit assignment** — `task.assignee` names the target agent directly.
2. **Capability matching** — match task content against agent skill tags and `AgentCapabilities` (see ADR-015).
3. **Round-robin** — among capable connected agents, select the one with the fewest active tasks.

### Implementation

```rust
pub trait TaskRouter: Send + Sync {
    fn select_agent(
        &self,
        task: &Task,
        registry: &AgentRegistry,
        skills: &SkillRegistry,
    ) -> Option<AgentId>;
}
```

`DefaultTaskRouter` implements this trait. The router:

- Only considers agents with `AgentStatus::Connected`.
- Is stateless — reads the agent registry and task manager at call time.
- Returns `None` if no suitable agent is found (task stays in `Submitted`).

When no agent matches, the task remains queued. The `TaskFacade` (ADR-014) re-evaluates routing when a new agent registers via bus events.

## Consequences

- Decoupled routing enables pluggable strategies without changing core task logic.
- Default strategy is simple and predictable.
- Tasks may wait if no agent matches — callers should implement timeouts.
- Future extensions (priority queues, load-based routing, agent affinity) implement the same trait.
