# ADR-006: Skill Registry -- Tools as First-Class Resources

## Status

Accepted

## Context

The nexus needs to expose concrete, callable tools ("skills") that agents can use. These skills may be provided by other agents, by the nexus itself, or registered by users on behalf of agents. We need a unified registry where skills are registered, discovered, and invoked -- regardless of who provides them or which protocol the caller uses.

## Decision

We will implement a **skill registry** as a core component. A skill is a concrete, callable tool with:

- **Name**: unique identifier (e.g. `code.search`, `git.diff`).
- **Description**: human/agent-readable explanation of what the skill does.
- **Input schema**: JSON Schema defining the expected parameters.
- **Output schema**: JSON Schema defining the return type.
- **Provider**: the agent or system component that implements the skill.

### Registration

Skills can be registered through any transport:

- **Agent-registered**: an agent connects and advertises skills it provides.
- **User-registered**: a user configures skills via CLI or config file (e.g. pointing to an external API or script).
- **Built-in**: the nexus ships with core skills (agent listing, task status, etc.).

### Discovery and Invocation

- Over **MCP**: skills are exposed as MCP tools. When skills change, the server emits `notifications/tools/list_changed` so clients refresh their tool list.
- Over **gRPC**: the A2A proto includes `ListSkills` and `InvokeSkill` RPCs.
- Over **REST**: `GET /skills` and `POST /skills/{name}/invoke`.

### Invocation flow

```text
Caller ──▶ Transport Adapter ──▶ Skill Registry ──▶ Provider Agent
                                    (resolve)         (execute)
```

The registry resolves the skill to its provider and routes the invocation. If the provider is a remote agent, the nexus proxies the call over whatever transport that agent is connected on.

## Consequences

- Skills are protocol-agnostic. An agent registering a skill over gRPC can have that skill invoked by an MCP client, and vice versa.
- The nexus becomes a skill broker, adding a hop but enabling cross-protocol interoperability.
- We need to handle provider unavailability gracefully (timeouts, retries, fallback).
- Skill naming needs conventions to avoid collisions (namespacing by agent or domain).
