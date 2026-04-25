# ADR-004: Agent-to-Agent (A2A) Protocol via gRPC

## Status

Accepted

## Context

Agents need a structured, strongly-typed protocol for communication. REST/JSON is flexible but lacks schema enforcement and efficient serialization. We need a protocol that supports streaming, bidirectional communication, and code generation across languages.

## Decision

We will define the agent-to-agent protocol using Protocol Buffers (`proto/a2a.proto`) and generate Rust code using protobuf/gRPC codegen (e.g. `tonic-build` or similar). This protocol covers:

- Agent registration and heartbeat.
- Task assignment and status reporting.
- Inter-agent message passing.
- Capability advertisement.

The gRPC server will run alongside the axum HTTP server, either on a separate port or multiplexed.

## Consequences

- Strong typing and schema evolution via protobuf.
- gRPC gives us streaming (server, client, and bidirectional) out of the box.
- Code generation means other languages can implement agents that connect to the nexus.
- We add protoc as a build dependency, which complicates the build pipeline slightly.
- We need to decide how gRPC and HTTP/axum coexist (separate ports vs. multiplexing).
