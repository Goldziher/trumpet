# Trumpet

Agent nexus -- a persistent background service that orchestrates
multiple AI agents through shared state, tools, and conversations.

## Overview

Agents in different terminals (Claude Code, Codex, etc.) connect to a
trumpet daemon and communicate through shared conversations. The daemon
handles agent registration, messaging, and real-time event delivery.

## Installation

```sh
cargo install --path .
```

## Quick start

Start the daemon:

```sh
trumpet serve
```

Check status:

```sh
trumpet status
```

Register an agent:

```sh
curl --unix-socket ~/.trumpet/trumpet.sock \
  http://localhost/agents/register \
  -H 'Content-Type: application/json' \
  -d '{"name": "alice"}'
```

Create a conversation and send a message:

```sh
curl --unix-socket ~/.trumpet/trumpet.sock \
  http://localhost/conversations \
  -H 'Content-Type: application/json' \
  -d '{"participants": ["AGENT_ID_HERE"]}'

curl --unix-socket ~/.trumpet/trumpet.sock \
  http://localhost/conversations/CONV_ID/messages \
  -H 'Content-Type: application/json' \
  -d '{"sender": "AGENT_ID_HERE", "content": "hello"}'
```

Subscribe to events (SSE):

```sh
curl --unix-socket ~/.trumpet/trumpet.sock http://localhost/events
```

## Security

The daemon enforces local-process authentication on every protocol surface
(see [ADR-017](adrs/017-local-auth.md)):

- **REST + WebSocket** — connections to the Unix socket are accepted only
  from processes whose UID matches the daemon owner. Foreign UIDs are
  dropped at accept time.
- **gRPC** — every request must include `Authorization: Bearer <token>`.
  The token is auto-generated on first start and written to
  `~/.trumpet/auth.token` with `0600` permissions; copy it from there
  into your client.

The daemon emits a startup warning if `server.host` is bound to a
non-loopback address; bearer tokens alone are not a TLS substitute over
the network.

Two further protections worth knowing about:

- **HTTP body limit.** Requests larger than 1 MiB are rejected with
  HTTP 413 to bound memory pressure from oversized payloads.
- **`code_tools` sandbox.** The built-in `code.scan_repo`,
  `code.read_file`, and `code.parse_file` tools resolve all paths
  against `code_tools.workspace_root` (defaulting to the daemon's CWD).
  Path traversal, absolute paths outside the root, and symlink escapes
  are rejected.

## Architecture

See [architecture.md](architecture.md) for the full design.
Architectural decisions are documented in [adrs/](adrs/).

## Development

```sh
task setup    # Install deps and pre-commit hooks
task build    # Build
task test     # Run tests
task lint     # Clippy + fmt checks
task coverage # Coverage report
```

## License

TBD
