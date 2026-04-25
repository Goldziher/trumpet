# Trumpet

Agent nexus -- a persistent background service that orchestrates
multiple AI agents through shared state, skills, and conversations.

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
