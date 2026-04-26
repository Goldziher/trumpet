# ADR-018: Tool Registry Notifications via `list_changed`

## Status

Accepted

## Context

ADR-016 introduced the unified tool registry. Tools enter the registry when
agents register (advertising their own capabilities) and leave when agents
disconnect or tools are explicitly deregistered. MCP clients need to discover
the current tool set dynamically so the LLM is never asked to call a tool
that no longer exists.

The MCP spec defines `notifications/tools/list_changed` for this. Each MCP
client runs in its own session with its own `rmcp::Peer` — broadcasting a
single global notification to all sessions is wrong (it couples the registry
to MCP transport details and races with sessions that join later).

## Decision

When the tool registry mutates, the bus emits `Event::ToolRegistered` /
`Event::ToolDeregistered`. A per-session notifier task (`src/mcp/notifier.rs`)
subscribes to the bus, debounces 100 ms, and emits
`notifications/tools/list_changed` on its session's `Peer`. The task is
spawned once per MCP session in the `on_initialized` handler
(`src/mcp/handler.rs`) and exits when the peer disconnects.

The 100 ms debounce window collapses bursts (tens of registrations during
agent startup) into a single notification.

## Consequences

### Easier

- Tool changes propagate to MCP clients automatically.
- Each session is independent — a slow client cannot back-pressure others.
- The bus stays the single source of truth for registry state.

### Harder

- One tokio task per active MCP session. For a daemon hosting hundreds of
  long-lived MCP sessions this adds up, but it is still bounded by the
  number of MCP sessions, which we expect to be modest.
- Clients see at least 100 ms of latency between a registration and the
  matching notification. Fine for interactive agents.

### Out of scope

- Per-tool `tool_added` / `tool_removed` notifications. Clients re-fetch
  the full list on each `list_changed`, which is acceptable at the volumes
  we expect.
- Notification ordering across sessions. Each session's notifier emits in
  bus order; sessions do not coordinate.
