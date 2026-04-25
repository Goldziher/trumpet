# ADR-010: TOML-Based Configuration with Strong Validation

## Status

Accepted

## Context

The daemon needs configuration for server ports, storage backends, agent defaults, skill paths, logging levels, and more. We need a format that is human-readable, well-structured, and widely supported. Configuration should be the primary mechanism for controlling behavior -- prefer config over code changes or environment variables.

## Decision

We will use TOML as the configuration format, loaded from `~/.trumpet/config.toml` (user-level) and `./trumpet.toml` (project-level, optional override).

### Structure

```toml
[daemon]
socket = "~/.trumpet/trumpet.sock"
pid_file = "~/.trumpet/trumpet.pid"

[server]
host = "127.0.0.1"
http_port = 7600
grpc_port = 7601

[mcp]
transport = "stdio"  # "stdio" | "http"

[storage]
backend = "fs"
path = "~/.trumpet/state"
snapshot_interval_secs = 60

[logging]
level = "info"
format = "json"  # "json" | "pretty"

[agents.defaults]
heartbeat_interval_secs = 30
timeout_secs = 300
```

### Validation

- All configuration is deserialized into strongly-typed Rust structs using `serde` + `toml`.
- Validation is performed at load time, before the daemon starts. Invalid config is a hard error with clear diagnostics.
- Use `#[validate]` attributes (via a validation crate or custom derive) to enforce constraints: port ranges, valid paths, non-empty strings, enum variants, duration bounds, etc.
- Config structs are the single source of truth for what is configurable and what the defaults are.

### Layering

Config is resolved in order (later overrides earlier):

1. Compiled-in defaults (struct field defaults).
2. `~/.trumpet/config.toml` (user-level).
3. `./trumpet.toml` (project-level).
4. Environment variables (`TRUMPET_*` prefix, mapped to config keys).
5. CLI flags (highest precedence).

### Config module

A dedicated `src/config/` module owns:

- Typed config structs (one per section).
- Loading, layering, and validation logic.
- A `Config::load()` entrypoint that returns a validated, immutable config or a structured error.

## Consequences

- TOML is familiar and well-supported in the Rust ecosystem.
- Strong typing + validation means invalid configs fail fast with clear messages rather than causing runtime surprises.
- Configuration-first approach means behavior changes rarely require code changes.
- Layering provides flexibility: global defaults, per-user settings, per-project overrides, and CLI escape hatches.
- We must keep config structs, defaults, and documentation in sync as features evolve.
