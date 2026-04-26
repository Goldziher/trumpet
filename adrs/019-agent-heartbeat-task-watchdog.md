# ADR-019: Agent Liveness via Heartbeat and Task Watchdog

## Status

Accepted

## Context

ADR-001 modelled agents with a four-state lifecycle (Connected, Idle, Busy,
Disconnected) but only the manual `deregister` path could move an agent to
`Disconnected`. Agents that crash or lose their network silently keep their
`Connected` status. Tasks routed to a dead agent never start, and there is
no mechanism to fail them.

Tasks themselves have the same problem: ADR-012 defines the lifecycle, but
without a deadline an in-flight task can hang forever.

## Decision

Add a per-process watchdog task (`src/core/watchdog.rs`) and an explicit
heartbeat surface:

1. Each `AgentInfo` tracks `last_heartbeat_at`, set on register and on every
   `POST /agents/{id}/heartbeat` (REST) or `heartbeat_agent` MCP-tool call.
2. The watchdog ticks every `agents.heartbeat_interval_secs` (default 5 s).
   Agents whose `last_heartbeat_at` is older than `agents.timeout_secs`
   (default 30 s) flip to `Disconnected`, emitting
   `Event::AgentDisconnected`.
3. In-flight tasks assigned to a newly-disconnected agent auto-fail with
   reason "assignee disconnected".
4. Tasks have an optional `deadline: DateTime<Utc>`. The watchdog also
   sweeps tasks past their deadline and transitions
   Submitted → Rejected, Working/Interrupted → Failed.

The router rejects task submissions pinned to an agent that is currently
`Disconnected` with a structured `Error::AgentDisconnected`.

A single watchdog (rather than per-agent or per-task timers) keeps the
wakeup count constant regardless of agent / task volume.

## Consequences

### Easier

- Dead agents are detected within `timeout_secs`; their work doesn't
  silently hang.
- Callers can bound task latency with `deadline_ms` / `deadline_at` on
  `POST /tasks`.
- One background task scales to thousands of agents with no timer churn.

### Harder

- Agents must heartbeat. SDK helpers should hide this behind a periodic
  caller; agents that forget will be marked dead.
- Task deadlines are wall-clock. Clock skew between caller and daemon will
  surprise users — document that the daemon's clock is authoritative.

### Out of scope

- Reassigning a disconnected agent's tasks to a different agent. We fail
  rather than reassign because tasks may have side effects we cannot
  reproduce.
- Distinguishing genuine deadness from GC pauses or temporary network
  hiccups. Agents that need a longer grace period can raise
  `agents.timeout_secs`.
