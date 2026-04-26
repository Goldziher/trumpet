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

Start the daemon (in another terminal):

```sh
trumpet start          # daemonize
trumpet status
```

Register an agent and submit a task:

```sh
trumpet agent register --name alice --tags review,lint
TASK=$(trumpet task submit --message "review my PR" --json | jq -r .id)
trumpet task get "$TASK"
```

Tail the live event stream while you work:

```sh
trumpet events tail &
```

Open a conversation between two agents:

```sh
trumpet chat new --participants ALICE_ID,BOB_ID
trumpet chat send CONV_ID --as ALICE_ID --content "hello bob"
trumpet chat history CONV_ID
```

Use `trumpet --help` and `trumpet <subcommand> --help` for the full
reference; every read command supports `--json` for scripting.

## CLI reference

The `trumpet` binary is both the daemon and the client:

| Command                                                     | Purpose                                       |
|-------------------------------------------------------------|-----------------------------------------------|
| `trumpet serve` / `trumpet start` / `trumpet stop`           | Run / daemonize / shut down the daemon        |
| `trumpet status`                                             | Daemon health and connected-agent summary     |
| `trumpet agent {register\|list\|get\|deregister\|heartbeat}` | Agent lifecycle                               |
| `trumpet task {submit\|list\|get\|cancel\|watch}`            | A2A task lifecycle (submit, watch, cancel)    |
| `trumpet tool {list\|invoke}`                                | Tool registry discovery and invocation        |
| `trumpet chat {new\|list\|send\|history}`                    | Conversation management                       |
| `trumpet events tail`                                        | Stream all daemon events as SSE               |
| `trumpet auth {show\|rotate}`                                | Show or rotate the gRPC bearer token          |

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

### Token rotation

Rotate the gRPC bearer token without restarting the daemon (ADR-021):

```sh
trumpet auth rotate
```

This generates a new UUID v4, atomically writes it to
`~/.trumpet/auth.token` (mode 0600 preserved), and signals the running
daemon via SIGHUP. The daemon swaps the new value into the bearer-token
interceptor's `Arc<RwLock<String>>`. In-flight requests finish with the
old token; new requests must present the new one.

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
