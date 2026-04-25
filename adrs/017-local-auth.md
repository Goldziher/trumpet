# ADR-017: Local-process authentication

## Status

Accepted

## Context

The Trumpet daemon binds three protocol surfaces:

- A Unix domain socket at `~/.trumpet/trumpet.sock` for REST + WebSocket.
- A TCP gRPC listener on `127.0.0.1:7601` (configurable) for the A2A
  protocol.
- A stdio MCP server when launched in an MCP-compatible host.

Without authentication, **any local process** with filesystem access to the
socket — or any process able to dial localhost — can register agents,
invoke arbitrary tools, cancel tasks, eavesdrop on event streams, and read
files reachable through the code-tools sandbox. For a daemon that
orchestrates AI agents on a developer machine that bar is too low.

Three options were considered:

1. **Skip auth, document the risk.** Cheapest. Leaves a P0 attack surface
   that any user-level malware can exploit; rejected.
2. **mTLS for gRPC + UNIX peer-cred for REST.** Most defensible across
   network boundaries but requires cert generation, rotation, and a UI
   for the cert path. Overkill for a single-user local daemon.
3. **UNIX peer-credential check + shared bearer token.** Matches the
   threat model — the daemon is always co-located with its caller. The
   token is filesystem-protected (`0600`) and never transmitted off-host.

## Decision

Adopt option 3.

REST and WebSocket connections arrive over a Unix domain socket; we wrap
the `tokio::net::UnixListener` in a `PeerCheckedUnixListener` that calls
`peer_cred()` on every accepted stream and drops connections whose UID
does not match the daemon owner's. The check is enforced **at accept
time**, before any HTTP framing is read, so unauthorized callers cannot
even probe the request surface.

gRPC arrives over TCP and has no equivalent peer credential. We require a
shared bearer token presented in the `Authorization: Bearer <token>`
metadata field, enforced by a `tonic::service::Interceptor`. The token
itself is a UUID v4 (122 bits of entropy), generated on first daemon
startup and written to `~/.trumpet/auth.token` with `0600` permissions.
Any client running as the daemon owner can read the file; any other UID
cannot.

A `[security]` config section exposes:

- `auth_token_path` — path to the token file (default `~/.trumpet/auth.token`).
- `require_auth` — when `false`, both checks are skipped. Intended for
  test harnesses and local development where the daemon is intentionally
  open. The daemon logs a WARN at startup when this is set.

The daemon also warns at startup when `server.host` is `0.0.0.0` or `::`,
since the bearer token alone is not a TLS substitute over a routable
network.

## Consequences

### Easier

- A foreign UID cannot interact with the daemon at all over REST/WS,
  even if it discovers the socket path.
- A foreign process on the same host (e.g. browser-based malware running
  in user space) cannot dial the gRPC port without first reading the
  token file, which is mode-locked.
- The cost of the check is one `peer_cred` syscall per connection plus
  one string comparison per gRPC request — negligible.
- A future move to mTLS only changes the gRPC layer; the REST layer
  remains correct because peer-cred is unaffected.

### Harder

- gRPC clients now need to load the token file themselves. Documented in
  the README and surfaced via the `trumpet status` CLI command.
- The token file becomes part of the daemon's persistent state. If the
  user deletes it the daemon regenerates a fresh token on next start,
  invalidating any clients still holding the old one.
- We have no token rotation story yet. For v0.1 the recommended
  rotation procedure is to stop the daemon, delete the token file, and
  restart — clients then re-read.

### Not addressed

- No protection against an attacker who already has full read access to
  the daemon owner's home directory (they can read the token, the
  config, and the snapshot key). This is consistent with the threat
  model: the daemon owns the user's data.
- Per-tool authorization. Once a request is authenticated it can invoke
  any registered tool. Fine-grained capabilities can layer on top in a
  future ADR.
- Network mTLS for cross-host gRPC. Out of scope for a local daemon.
