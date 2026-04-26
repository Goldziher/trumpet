# ADR-021: Full CLI Client Surface and SIGHUP Auth Rotation

## Status

Accepted

## Context

Pre-v0.1.0 the CLI was operator-only: `serve`, `start`, `stop`, `status`.
Anyone wanting to drive the daemon — register an agent, submit a task,
invoke a tool, tail events — had to script `curl --unix-socket` against
the REST surface. ADR-002 always intended the CLI to be a full client too
("management, diagnostics, and one-off operations"); this ADR closes the
gap.

Auth rotation had a related gap: ADR-017 left rotation as "stop the
daemon, delete the token, restart". That is unacceptable for a long-lived
nexus that other agents are connected to.

## Decision

### CLI client commands

Add six client subcommand groups (`src/cli/{agent,task,tool,chat,events,auth}.rs`):

- `trumpet agent {register|list|get|deregister|heartbeat}`
- `trumpet task {submit|list|get|cancel|watch}`
- `trumpet tool {list|invoke}`
- `trumpet chat {new|list|send|history}`
- `trumpet events tail [--filter ...]`
- `trumpet auth {show|rotate}`

Read commands take `--json` for scripting. Shared transport in
`src/cli/client.rs` writes raw HTTP/1.1 over the daemon's Unix socket and
parses responses inline — no `reqwest` / `hyper` client dep on the CLI
side, matching the existing `cli::status` pattern. Streaming commands
(`task watch`, `events tail`) parse SSE frames line-by-line so chunked
transfer encoding does not break framing.

### SIGHUP auth rotation

`trumpet auth rotate`:

1. Generate a new UUID v4.
2. Write it to a temp file in `~/.trumpet/`, fsync, atomic-rename over
   `auth.token` (mode 0600 preserved).
3. Read the daemon PID from the PID file.
4. Send SIGHUP to that PID.
5. Print the new token.

The daemon's bearer-token interceptor now reads from
`Arc<RwLock<String>>` instead of `Arc<String>`. A SIGHUP handler installed
in `serve_with_shutdown` re-reads `auth.token` and swaps the new value
into the same lock. Existing gRPC connections finish their current
request with the old token; new requests must present the new token.

A side-fix: the daemon now actually writes its PID file at startup (and
removes it on shutdown). `trumpet stop` and `trumpet auth rotate` both
need it.

## Consequences

### Easier

- Users drive the daemon from one binary. No `curl --unix-socket` calls.
- Token rotation is online — no daemon restart, no listener bounce, no
  client outage beyond a single rejected request retry.
- `--json` makes every command pipeable into `jq` / shell scripts.

### Harder

- The CLI is now a full HTTP/1.1 client. We hand-roll request building
  and SSE framing; bugs there look like daemon bugs to the user.
- Bearer-token interceptor takes a sync `RwLock` (because tonic's
  `Interceptor::call` is sync). A poisoned lock (panic in another holder)
  fails open — the interceptor returns 500 — but cannot wedge the
  daemon.

### Out of scope

- Multi-token authorization (one token per client). Single token shared
  across clients matches ADR-017.
- Token TTL or auto-rotation. v0.1.0 rotation is user-initiated.
- Shell completion. Trivial to add via clap; deferred until requested.
