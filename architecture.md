# Trumpet Architecture

## Overview

Trumpet is an **agent nexus** -- a persistent background service that coordinates multiple AI agents. It runs as a daemon, allowing agents in different terminals (e.g. Claude Code, Codex) to share state, tools, and conversations through a common substrate. It provides agent discovery, task routing, state management, and inter-agent communication through multiple protocol interfaces.

## High-Level Architecture

```text
  Terminal 1            Terminal 2            Terminal N
┌──────────┐          ┌──────────┐          ┌──────────┐
│  Claude  │          │  Codex   │          │  Agent   │
│  Code    │          │          │          │          │
└────┬─────┘          └────┬─────┘          └────┬─────┘
     │ MCP                 │ MCP/gRPC            │ any
     └─────────┬───────────┘─────────────────────┘
               │
┌──────────────▼──────────────────────────────────────┐
│              trumpet daemon (background)             │
│                                                      │
│  ┌──────────┐  ┌──────────┐  ┌───────────────────┐   │
│  │   CLI    │  │  axum    │  │   gRPC (A2A)      │   │
│  │  (clap)  │  │  server  │  │   (tonic)         │   │
│  └────┬─────┘  └────┬─────┘  └────────┬──────────┘   │
│       │              │                 │              │
│       │         ┌────┴─────┐           │              │
│       │         │   MCP    │           │              │
│       │         │  server  │           │              │
│       │         └────┬─────┘           │              │
│       │              │                 │              │
│  ┌────▼──────────────▼─────────────────▼───────────┐  │
│  │                   Core                          │  │
│  │                                                 │  │
│  │  ┌────────────┐ ┌──────────┐ ┌───────────────┐  │  │
│  │  │  Agent     │ │  Task    │ │  Message      │  │  │
│  │  │  Registry  │ │  Router  │ │  Bus          │  │  │
│  │  └────────────┘ └──────────┘ └───────────────┘  │  │
│  │  ┌────────────┐ ┌───────────────────────────┐   │  │
│  │  │  Tool      │ │  Chat / Conversation      │   │  │
│  │  │  Registry  │ │  Manager                  │   │  │
│  │  └────────────┘ └───────────────────────────┘   │  │
│  │                                                 │  │
│  └──────────────────────┬──────────────────────────┘  │
│                         │                             │
│  ┌──────────────────────▼──────────────────────────┐  │
│  │              State Manager                      │  │
│  │  in-memory state + write-behind persistence     │  │
│  │                                                 │  │
│  │              ┌─────────────┐                    │  │
│  │              │   OpenDAL   │                    │  │
│  │              │  Operator   │                    │  │
│  │              └──────┬──────┘                    │  │
│  │                     │                           │  │
│  │         fs | s3 | gcs | redis | ...             │  │
│  └─────────────────────────────────────────────────┘  │
└───────────────────────────────────────────────────────┘
```

## Components

### Interface Layer

- **CLI (clap)**: Command-line interface for management, diagnostics, and one-off operations.
- **HTTP Server (axum)**: REST and WebSocket APIs for agent communication and monitoring.
- **MCP Server (rust-sdk)**: Model Context Protocol server exposing nexus capabilities as tools/resources to MCP-compatible clients.
- **gRPC Server (tonic)**: Agent-to-agent protocol defined in `proto/a2a.proto` for structured, streaming agent communication.

### Core

- **Agent Registry**: Tracks registered agents, their capabilities, and lifecycle state (idle, busy, error, disconnected). Each agent carries `last_heartbeat_at`, bumped by `POST /agents/{id}/heartbeat` (REST) or the `heartbeat_agent` MCP tool.
- **Task Router**: Matches incoming tasks to agents based on capability requirements and agent availability (ADR-013, ADR-015). Tasks may carry a `deadline`; the watchdog enforces it.
- **Watchdog**: Background task (`src/core/watchdog.rs`, ADR-019) that ticks every `agents.heartbeat_interval_secs`, flips agents past their `agents.timeout_secs` to `Disconnected`, fails their in-flight tasks, and fails any task past its `deadline`.
- **Message Bus**: Internal broadcast system (`tokio::sync::broadcast`) for event propagation. Each transport adapter subscribes and forwards events in its native format.
- **Tool Registry**: Manages callable tools that agents provide and consume (ADR-016). Tools are registered by agents, users, or built-in. Exposed as MCP tools, gRPC RPCs, and REST endpoints. Emits `notifications/tools/list_changed` to MCP sessions when tools are added or removed (ADR-018).
- **Chat / Conversation Manager**: Manages chat-like message threads between agents. Agents can open conversations, send messages, and subscribe to updates -- enabling collaborative, multi-turn interactions between agents.
- **State Manager**: In-memory state with write-behind persistence via OpenDAL. Supports snapshot + journal for crash recovery. Default backend is `fs` at `~/.trumpet/state/`, configurable to any OpenDAL-supported backend. Snapshots are encrypted at rest (ADR-017).

## Daemon Lifecycle

| Command                        | Description                                       |
|--------------------------------|---------------------------------------------------|
| `trumpet serve`                | Start daemon in foreground                        |
| `trumpet start`                | Start daemon in background (daemonize)            |
| `trumpet stop`                 | Graceful shutdown (flush state, disconnect agents)|
| `trumpet status`               | Show daemon status and connected agents           |
| `trumpet agent {register|list|get|deregister|heartbeat}` | Agent lifecycle management              |
| `trumpet task {submit|list|get|cancel|watch}`            | Task lifecycle management               |
| `trumpet tool {list|invoke}`                             | Tool discovery and invocation           |
| `trumpet chat {new|list|send|history}`                   | Conversation management                 |
| `trumpet events tail`                                    | Live SSE stream                         |
| `trumpet auth {show|rotate}`                             | Auth-token introspection / rotation     |

`trumpet auth rotate` writes a new UUID v4 token to `~/.trumpet/auth.token`
and signals the running daemon via SIGHUP; the daemon re-reads the file
without restarting any listener (ADR-021).

The daemon binds to `~/.trumpet/trumpet.sock` (unix domain socket). CLI commands and agents discover the daemon via this well-known path.

## Protocol Interfaces

| Interface | Transport                          | Purpose                                    |
|-----------|-----------------------------------|--------------------------------------------|
| CLI       | stdin/stdout                       | Human operator interaction                 |
| REST      | HTTP/1.1 over Unix socket (axum)   | Task submission, monitoring, agent admin   |
| WebSocket | HTTP upgrade                       | Live updates, streaming results            |
| MCP       | stdio                              | In-process LLM/agent integration           |
| MCP       | streamable-HTTP at `/mcp`          | Out-of-process MCP clients (rmcp)          |
| A2A gRPC  | HTTP/2 with bearer-token auth      | Agent registration, messaging, streaming   |

The MCP HTTP transport is mounted under the same Unix-socket axum router
as the REST API, so it inherits the peer-credential auth check (ADR-020).
The gRPC server binds a separate loopback TCP port; the bearer token is
generated at first startup and stored in `~/.trumpet/auth.token` with
mode 0600. Token rotation is online via SIGHUP (ADR-021).

## Crate Structure

```text
trumpet/
├── src/
│   ├── main.rs          # Entrypoint, clap dispatch
│   ├── config/          # TOML config structs, loading, validation
│   ├── cli/             # CLI commands
│   ├── server/          # axum server, routes, WebSocket
│   ├── mcp/             # MCP server integration
│   ├── grpc/            # gRPC service implementations
│   ├── core/            # Agent registry, task router, message bus
│   │   ├── registry.rs  # Agent registry
│   │   ├── router.rs    # Task router
│   │   ├── bus.rs       # Message bus (broadcast)
│   │   ├── tools.rs     # Tool registry
│   │   └── chat.rs      # Conversation manager
│   ├── state/           # State manager, OpenDAL integration
│   └── daemon/          # Process management, PID file, signals
├── proto/
│   └── a2a.proto        # Agent-to-agent protocol definition
├── trumpet.toml         # Project-level config (optional)
├── adrs/                # Architecture Decision Records
└── architecture.md      # This file
```

## Key Design Principles

1. **Core independence**: The core module has no knowledge of transport layers. It exposes a Rust API that CLI, HTTP, MCP, and gRPC adapters call into.
2. **Async-first**: Built on tokio. All I/O is async. The core uses channels and async primitives for internal communication.
3. **Protocol polyglot**: Agents can connect via whichever protocol suits them -- gRPC for structured comms, MCP for LLM integration, REST/WebSocket for simplicity.
