# ADR-001: Agent Nexus as Core Architecture

## Status

Accepted

## Context

We need a system that orchestrates multiple AI agents -- coordinating task routing, managing agent state, and handling inter-agent communication. This system, which we call an "agent nexus", acts as the central hub through which agents discover each other, exchange messages, and collaborate on complex tasks.

## Decision

We will build an agent nexus in Rust (edition 2024). The nexus is responsible for:

- **Agent registration and discovery**: agents register with the nexus and can discover other agents by capability.
- **Task routing**: incoming tasks are routed to the appropriate agent(s) based on capability matching.
- **State management**: the nexus tracks agent lifecycle (idle, busy, error) and task progress.
- **Inter-agent communication**: agents communicate through the nexus, enabling collaboration on multi-step tasks.

The project is named `trumpet` (crate name).

## Consequences

- We take on the complexity of building an orchestration layer, but gain full control over agent coordination semantics.
- Rust gives us memory safety, high performance, and a strong type system for modeling agent protocols.
- Edition 2024 gives us access to the latest language features but limits us to recent toolchain versions.
