# ADR-008: Background Daemon with Agent Bridging

## Status

Accepted

## Context

We want to run multiple AI agents simultaneously -- e.g. Codex in one terminal and Claude Code in another -- and have them collaborate through the nexus. This requires the nexus to run as a long-lived background service (daemon) that survives individual agent sessions. Agents connect and disconnect freely; the nexus maintains shared state across all of them.

## Decision

We will implement a **daemon mode** for trumpet:

### Lifecycle

- `trumpet serve` starts the daemon in the foreground.
- `trumpet start` starts the daemon in the background (daemonizes).
- `trumpet stop` gracefully shuts down the daemon.
- `trumpet status` reports whether the daemon is running and lists connected agents.

The daemon binds to a local socket (unix domain socket or localhost TCP port). A PID file and/or socket file at a well-known path (e.g. `~/.trumpet/trumpet.sock`) allows CLI commands and agents to discover the running daemon.

### Agent bridging

Each connected agent is a peer in the nexus. The daemon enables:

- **Shared conversations**: Claude and Codex can participate in the same chat thread (ADR-007).
- **Shared skill access**: a skill registered by one agent is callable by the other.
- **Shared task state**: both agents see the same task board and can pick up, hand off, or collaborate on tasks.
- **Live updates**: both agents receive real-time notifications of changes made by the other.

### Connection model

Agents connect to the daemon via MCP (stdio proxy or HTTP), gRPC, or WebSocket. The daemon treats all connections equally -- it doesn't matter which agent is which, only what capabilities they advertise.

```text
Terminal 1              Terminal 2
┌──────────┐            ┌──────────┐
│  Claude  │            │  Codex   │
│  Code    │            │          │
└────┬─────┘            └────┬─────┘
     │ MCP                   │ MCP/gRPC
     │                       │
┌────▼───────────────────────▼─────┐
│         trumpet daemon           │
│     (background service)         │
│                                  │
│  shared state, skills, chat      │
└──────────────────────────────────┘
```

## Consequences

- The nexus is no longer ephemeral -- it holds state that outlives any single agent session.
- We need process management: PID files, graceful shutdown, signal handling (SIGTERM, SIGHUP).
- The daemon must handle agent disconnects without losing state.
- Unix domain sockets give us fast local IPC with filesystem-based access control.
- We need a state persistence layer (see ADR-009) since the daemon itself may restart.
