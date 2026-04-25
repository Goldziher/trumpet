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

- **Agent Registry**: Tracks registered agents, their capabilities, and lifecycle state (idle, busy, error, disconnected).
- **Task Router**: Matches incoming tasks to agents based on capability requirements and agent availability.
- **Message Bus**: Internal broadcast system (`tokio::sync::broadcast`) for event propagation. Each transport adapter subscribes and forwards events in its native format.
- **Tool Registry**: Manages callable tools that agents provide and consume (ADR-016). Tools are registered by agents, users, or built-in. Exposed as MCP tools, gRPC RPCs, and REST endpoints. Emits `list_changed` notifications when tools are added or removed.
- **Chat / Conversation Manager**: Manages chat-like message threads between agents. Agents can open conversations, send messages, and subscribe to updates -- enabling collaborative, multi-turn interactions between agents.
- **State Manager**: In-memory state with write-behind persistence via OpenDAL. Supports snapshot + journal for crash recovery. Default backend is `fs` at `~/.trumpet/state/`, configurable to any OpenDAL-supported backend.

## Daemon Lifecycle

| Command            | Description                                      |
|--------------------|--------------------------------------------------|
| `trumpet serve`    | Start daemon in foreground                       |
| `trumpet start`    | Start daemon in background (daemonize)           |
| `trumpet stop`     | Graceful shutdown (flush state, disconnect agents)|
| `trumpet status`   | Show daemon status and connected agents          |

The daemon binds to `~/.trumpet/trumpet.sock` (unix domain socket). CLI commands and agents discover the daemon via this well-known path.

## Protocol Interfaces

| Interface | Transport      | Purpose                          |
|-----------|---------------|----------------------------------|
| CLI       | stdin/stdout  | Human operator interaction       |
| REST      | HTTP (axum)   | Task submission, monitoring      |
| WebSocket | HTTP upgrade  | Live updates, streaming results  |
| MCP       | stdio / HTTP  | LLM/AI model integration         |
| A2A gRPC  | HTTP/2        | Agent registration, messaging    |

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
