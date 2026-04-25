# ADR-016: Tool Registry — Unified Tool Use Across Protocols

## Status

Accepted

## Context

Trumpet orchestrates agents connected via different protocols (MCP, gRPC/A2A, REST). Each protocol has its own concept of callable tools:

- **MCP**: tools are defined with JSON Schema input/output, invoked via `tools/call`
- **A2A**: agents advertise skills in their AgentCard, work happens via task messages
- **REST**: skills are listed at `GET /skills`, invocation at `POST /skills/{name}/invoke` returns 501

The existing `SkillRegistry` stores metadata (name, description, schemas, provider) but has no invocation path. When an MCP-connected agent registers tools, those tools should be callable by any other agent regardless of protocol. When a gRPC agent advertises capabilities, those should appear as invocable tools to MCP clients.

The term "skill" in the existing code is interchangeable with "tool" — we standardize on **tool** as the user-facing concept, matching MCP and A2A terminology.

## Decision

We will evolve the existing `SkillRegistry` into a `ToolRegistry` that supports both registration and invocation. A tool is a callable unit of work with:

- **Name**: unique dot-namespaced identifier (e.g. `code.parse_file`, `agent.research`)
- **Description**: human/agent-readable explanation
- **Input schema**: JSON Schema for parameters
- **Output schema**: JSON Schema for return value
- **Provider**: who implements the tool — `BuiltIn`, `Agent { agent_id }`, or `Remote { url }`

### Invocation Flow

```text
Caller (MCP / REST / gRPC / internal)
  ↓ invoke(tool_name, input)
ToolRegistry
  ↓ resolve provider
  ├── BuiltIn → call local function (e.g. CodeTools)
  ├── Agent { agent_id } → create Task assigned to agent, await result
  └── Remote { url } → forward via HTTP/gRPC to external endpoint
```

### Key Design Points

1. **Built-in tools** (e.g. `code.scan_repo`) are registered at startup and execute in-process via `CodeTools`. Their invocation is synchronous (spawn_blocking for CPU work).

2. **Agent-provided tools** are registered when an agent connects and deregistered when it disconnects. Invocation creates an A2A Task (ADR-012) assigned to the providing agent. The caller receives the task ID and can poll or subscribe for results.

3. **Tool invocation returns a `ToolResult`** — either an immediate value (for built-in tools) or a task reference (for agent-provided tools that execute asynchronously).

4. **MCP tool changes emit `notifications/tools/list_changed`** so connected MCP clients refresh their tool list when tools are added or removed.

5. **The existing `SkillRegistry` is renamed to `ToolRegistry`**. The `SkillInfo` type becomes `ToolInfo`. The `SkillProvider` enum becomes `ToolProvider`. This is a breaking rename but aligns with industry terminology.

### ToolResult Type

```rust
pub enum ToolResult {
    /// Immediate result from a built-in tool.
    Immediate { output: serde_json::Value },
    /// Asynchronous result — a task was created and assigned.
    TaskCreated { task: Task },
}
```

### Registration Sources

- **Daemon startup**: built-in tools (code.scan_repo, code.read_file, code.parse_file)
- **Agent connect**: agent advertises tools in its registration message
- **Agent disconnect**: agent's tools are deregistered
- **Config file**: static tool definitions in `trumpet.toml` (future)

## Consequences

- Tools become the universal invocation abstraction. An MCP client doesn't need to know whether a tool runs locally or on a remote agent.
- Agent-provided tool invocations go through the task system, giving them lifecycle tracking, persistence, and streaming for free.
- Built-in tools bypass the task system for low-latency responses.
- Renaming Skill→Tool is a breaking change to the REST API and internal types. This is acceptable pre-1.0.
- MCP `notifications/tools/list_changed` makes tool discovery dynamic — clients always have an up-to-date tool list.
