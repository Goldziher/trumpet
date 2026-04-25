# ADR-002: CLI + Server Deployment Model

## Status

Accepted

## Context

The nexus needs to be both interactive (for development, debugging, and one-off operations) and long-running (for production orchestration). We need a unified binary that supports both modes.

## Decision

We will build a single binary that operates in two modes:

- **CLI mode**: using [clap](https://docs.rs/clap) for command parsing. Supports commands for agent management, task submission, configuration, and diagnostics.
- **Server mode**: using [axum](https://docs.rs/axum) as the HTTP framework. Exposes REST/WebSocket APIs for agent communication, task submission, and monitoring.

Both modes share the same core library. The binary entrypoint uses clap to dispatch between CLI commands and the `serve` subcommand which starts the axum server.

## Consequences

- Single binary simplifies deployment and distribution.
- clap gives us typed, well-documented CLI with auto-generated help.
- axum is built on tokio/tower/hyper, giving us a mature async HTTP stack with middleware composition.
- We must structure the crate so that core logic is independent of the interface layer (CLI vs server).
