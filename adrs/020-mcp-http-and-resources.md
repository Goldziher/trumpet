# ADR-020: MCP HTTP Transport, Resources, and Prompts

## Status

Accepted

## Context

ADR-003 promised that the MCP server would support stdio *and* HTTP, and
that tool / resource / prompt would all be exposed. Through pre-v0.1.0,
only stdio was wired (`mcp.transport = "http"` hard-errored at startup) and
only tools were exposed — resources and prompts were missing. MCP clients
that prefer HTTP (Claude Code, Codex over a TCP relay) had no path in, and
clients that wanted to introspect daemon state had to issue tool calls for
data that should live behind a `resources/read`.

## Decision

### Transport

Mount rmcp's streamable-HTTP service at `/mcp` on the daemon's existing
Unix-socket axum router. The HTTP transport therefore inherits the
peer-credential auth check from `PeerCheckedUnixListener`; no second
listener and no new auth path. The stdio transport stays available behind
`mcp.transport = "stdio"`.

Implementation lives in `src/mcp/http.rs`; wiring is in
`src/server/routes.rs::router_with_mcp` and `src/server/mod.rs`.

### Resources

Expose seven `trumpet://` URIs via `RoleServer::list_resources` /
`read_resource` (`src/mcp/resources.rs`):

- `trumpet://agents`, `trumpet://agents/{id}`
- `trumpet://conversations`, `trumpet://conversations/{id}`
- `trumpet://tasks`, `trumpet://tasks/{id}`
- `trumpet://tools`

Each URI resolves at fetch time against the live `AppState`, so MCP clients
always see current state without the daemon needing to push.

### Prompts

Three named templates rendered server-side against live state
(`src/mcp/prompts.rs`):

- `summarize_task(task_id)`
- `assign_task(task_description, required_tags?)`
- `inspect_conversation(conversation_id)`

The prompts return fully-rendered messages — the LLM doesn't need to
re-fetch state to produce a useful response.

## Consequences

### Easier

- HTTP MCP clients reuse the daemon's Unix-socket auth; no token plumbing.
- Resource URIs give clients a stable read surface that doesn't grow with
  every new tool.
- Prompts let agents call common nexus operations without the LLM having
  to author each prompt from scratch.

### Harder

- Resources are JSON-encoded snapshots. Large nexus state (tens of
  thousands of tasks) will blow up the response. Pagination is not yet
  implemented — see "out of scope" below.
- Three protocol surfaces (tools, resources, prompts) means three places
  to keep in sync when adding a new nexus capability.

### Out of scope

- Pagination on `read_resource`. The MCP spec supports it; we will add it
  if response sizes become a problem.
- TCP exposure of the HTTP MCP transport. v0.1.0 keeps it on the Unix
  socket so peer-cred auth applies; a separate ADR will cover network
  exposure with TLS / mTLS when needed.
- `resources/subscribe` notifications. Clients re-read resources on
  demand; subscribing to per-resource changes can layer on later.
