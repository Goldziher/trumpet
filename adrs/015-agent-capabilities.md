# ADR-015: Agent Capabilities & Agent Card

## Status

Accepted

## Context

The A2A protocol defines `AgentCard` — a self-describing manifest for an agent covering identity, skills, supported communication modes, and security requirements. Trumpet's agent registry currently tracks only basic metadata (name, status, registration time). Without capability information, the task router (ADR-013) cannot match work to agents. Capability-based routing is essential for the nexus to function as a work distributor.

## Decision

We will extend `AgentInfo` with optional `AgentCapabilities`:

```rust
pub struct AgentCapabilities {
    /// Media types the agent accepts as input (e.g. "text/plain", "application/json").
    pub supported_input_modes: Vec<String>,
    /// Media types the agent can produce as output.
    pub supported_output_modes: Vec<String>,
    /// Whether the agent supports streaming responses.
    pub streaming: bool,
    /// Tags describing the agent's areas of expertise (e.g. "code", "research").
    pub skill_tags: Vec<String>,
}
```

Capabilities are provided at registration time. Agents that don't specify capabilities can still receive explicitly-assigned tasks but won't match capability-based routing.

### Agent Card Derivation

The A2A `AgentCard` is derived on-the-fly when requested, combining:

- `AgentInfo` (identity, status)
- `AgentCapabilities` (modes, streaming, tags)
- Skills from `SkillRegistry` where `provider == Agent { agent_id }`

This avoids storing a separate card that could become stale. The existing `SkillRegistry` remains the canonical skill store — capabilities reference skill names/tags but don't duplicate schema definitions.

### Registration Changes

`AgentRegistry::register` gains an optional `capabilities` parameter. The gRPC `RegisterAgent` and MCP `register_agent` tool pass capabilities from the wire format. The REST `POST /agents/register` body gains an optional `capabilities` field.

## Consequences

- Agents advertise what they can do. The router uses this for intelligent task assignment.
- Backward compatible — existing agents without capabilities still work via explicit assignment.
- No duplication of skill schemas — the card is a read-only projection.
- New modes or tags can be added without schema migration.
