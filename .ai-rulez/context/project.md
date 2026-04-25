---
description: Project overview, architecture, and conventions for Trumpet
globs: ["**/*.rs", "**/*.toml", "**/*.proto"]
---

# Trumpet — Agent Nexus

A persistent background service that orchestrates multiple AI agents (Claude Code, Codex, etc.) through shared state, skills, and conversations.

See `architecture.md` for the full architecture and `adrs/` for decision records.

## Language & Toolchain

- Rust edition 2024.
- Clippy with warnings as errors: `cargo clippy -- -D warnings`.
- Format with `cargo fmt --check`.
- Pre-commit checks via prek: `prek run --all-files`.
- Task runner: `task` (taskfile.yaml).

## Configuration

TOML-based configuration. Prefer configuration over code changes for controlling behavior.

- Config files: `~/.trumpet/config.toml` (user), `./trumpet.toml` (project override).
- Layering order: compiled defaults → user config → project config → env vars (`TRUMPET_*`) → CLI flags.
- All config is deserialized into strongly-typed structs (`serde` + `toml`).
- Validate at load time. Invalid config = hard error with clear diagnostics. Never silently fall back.
- Config structs live in `src/config/`. One struct per config section.
- `Config::load()` is the single entrypoint: returns validated, immutable config or a structured error.

## Code Structure

Small, focused modules. No large files. Each module has a single responsibility. If a file grows beyond ~200 lines, split it.

```text
src/
├── main.rs            # Entrypoint, clap dispatch only
├── config/            # TOML config structs, loading, validation
├── cli/               # CLI commands (one file per subcommand)
├── server/            # axum routes, middleware, WebSocket
├── mcp/               # MCP server integration (rust-sdk)
├── grpc/              # gRPC service impls (tonic, codegen from proto/)
├── core/              # Domain logic, no transport awareness
│   ├── registry.rs    # Agent registry
│   ├── router.rs      # Task router
│   ├── bus.rs         # Message bus (tokio::sync::broadcast)
│   ├── skills.rs      # Skill registry
│   └── chat.rs        # Conversation manager
├── state/             # State manager, OpenDAL persistence
└── daemon/            # Process lifecycle, PID, signals
```

## Performance

Always prefer high-performance patterns where applicable:

- `ahash` over default `HashMap` hasher.
- `memchr` for byte/string searching.
- `Cow<'_, str>` to avoid unnecessary allocations.
- `Arc` for shared ownership across async tasks.
- SIMD-friendly data layouts where hot paths benefit.
- Zero-copy deserialization when parsing protocol messages.
- Prefer codegen (derive macros, proto codegen, build.rs) over hand-written boilerplate.

## Testing

We follow TDD. Write tests before or alongside implementation.

- Unit tests live in the same file as the code (`#[cfg(test)]` module).
- Integration tests live in `tests/`.
- Use coverage (`cargo llvm-cov` or `cargo tarpaulin`) to identify important logic that is untested.
- Every public function in `core/` must have tests.
- Use `tokio::test` for async tests.

## Build & Check Commands

```sh
task setup                           # Install deps and hooks
task build                           # Build
task lint                            # Pre-commit checks (prek)
task test                            # Run all tests
task coverage                        # Coverage report
task update                          # Update deps (cargo update + prek autoupdate)
task upgrade                         # Upgrade deps (cargo upgrade --incompatible + task update)
```

## Conventions

- Prefer `thiserror` for error types. Each module defines its own error enum.
- Use `tracing` for structured logging, not `println!` or `log`.
- All I/O is async (tokio). No blocking calls on the async runtime.
- Proto definitions live in `proto/`. Codegen outputs go to `OUT_DIR`, not checked in.
- Dependencies: use feature flags to keep compile times down (especially OpenDAL backends).
