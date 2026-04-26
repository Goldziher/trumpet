# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-04-26

First release of the Trumpet agent nexus.

### Added

- Background daemon with Unix-socket REST API, WebSocket / SSE event stream,
  gRPC A2A service, and dual-transport MCP server.
- Agent registry with capability-tagged lifecycle (`Connected` /
  `Disconnected`), heartbeat surface, and disconnection watchdog
  (ADR-001, ADR-019).
- Task system with state machine, deadlines, and watchdog auto-fail for
  stale tasks or disconnected assignees (ADR-012, ADR-019).
- Capability-aware task router supporting tag-based assignment (ADR-013,
  ADR-015).
- Tool registry exposed across MCP, gRPC, and REST, with
  `notifications/tools/list_changed` propagation to MCP sessions
  (ADR-016, ADR-018).
- MCP HTTP transport mounted at `/mcp` on the Unix socket plus
  resources (`trumpet://agents`, `trumpet://conversations`,
  `trumpet://tasks`, `trumpet://tools`) and prompts (`summarize_task`,
  `assign_task`, `inspect_conversation`) (ADR-003, ADR-020).
- gRPC A2A service with bearer-token auth and streaming RPCs
  (`SendStreamingMessage`, `SubscribeToTask`) (ADR-004).
- Binary message parts (`Part::Bytes`) round-trip through REST, gRPC,
  and persistence using base64 in JSON.
- Push-notification webhook delivery with retry/backoff for task
  lifecycle events.
- Tree-sitter-backed code-intelligence tools (`code.scan_repo`,
  `code.read_file`, `code.parse_file`) (ADR-011).
- Agent-to-agent chat / conversation manager with persistent history
  (ADR-007).
- Encrypted snapshot persistence over OpenDAL (`fs` / `s3` / `gcs` /
  `redis` / …) (ADR-009).
- Full CLI client surface — `agent`, `task`, `tool`, `chat`, `events`,
  `auth` subcommand groups with `--json` output (ADR-021).
- Architecture Decision Records ADR-001 through ADR-021.

### Security

- Peer-credential authentication on the Unix socket (UID match enforced
  at accept time).
- Bearer-token authentication on gRPC; token file mode 0600
  (ADR-017).
- Online auth-token rotation via `trumpet auth rotate` and SIGHUP — no
  daemon restart, no listener bounce (ADR-021).
- HTTP body limit (1 MiB) to bound memory pressure from oversized
  payloads.
- `code_tools` sandbox: workspace-root containment with rejection of
  path traversal, absolute paths outside the root, and symlink escapes.
