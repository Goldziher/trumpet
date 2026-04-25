# ADR-011: Tree-sitter Code Tools via tree-sitter-language-pack

## Status

Accepted

## Context

gitmind's core purpose is understanding code repositories. Agents operating through Trumpet need more than raw file bytes -- they need language-aware structure: what functions are defined, what a file imports, where a symbol is declared, and how a codebase is organized. Without this, every agent that wants code intelligence must implement its own ad-hoc parsing, which is fragile, inconsistent, and duplicates effort.

[tree-sitter](https://tree-sitter.github.io/) provides fast, incremental, error-tolerant parsing for hundreds of languages via a stable C API. It produces concrete syntax trees that can be queried with a pattern language, making it suitable for symbol extraction, pattern search, and structural analysis without requiring a full language server.

[tree-sitter-language-pack](https://github.com/kreuzberg-dev/tree-sitter-language-pack) (tslp) bundles pre-compiled grammars for 305 languages in a single Rust crate. Its `ts-pack-core` crate exposes:

- `LanguageRegistry` -- thread-safe parser lookup by language name or file extension.
- `process()` / `ProcessConfig` -- a unified pipeline producing `ProcessResult` with structure items, imports, exports, symbols, metrics, and doc comments.
- `run_query()` -- execute arbitrary tree-sitter queries against a parsed tree.
- Language detection from path, extension, or file content.
- `text_splitter` -- syntax-aware chunking for embedding or context-window management.

tslp already uses `ahash`, `memchr`, and `thiserror` -- consistent with our codebase conventions. Its static build (feature `default` without `dynamic-loading`) embeds all parsers in the binary, eliminating runtime download dependencies.

## Decision

We will integrate `tree-sitter-language-pack` (without the `dynamic-loading` or `download` features) as a core dependency and expose code analysis as built-in skills registered in the skill registry (ADR-006).

### New module: `src/core/code_tools.rs`

A `CodeTools` struct wraps a shared `Arc<LanguageRegistry>` and implements each skill as an async method that offloads CPU-bound tree-sitter work to `tokio::task::spawn_blocking`.

### Skills

All skills follow the naming convention `code.<verb>` and are registered as built-in skills at daemon startup.

| Skill name | Input | Output |
|---|---|---|
| `code.scan_repo` | `path: String`, `max_depth?: u32`, `include_patterns?: [String]`, `exclude_patterns?: [String]` | File listing with detected language, size, and line count per file |
| `code.read_file` | `path: String` | File content, detected language, and line count |
| `code.parse_file` | `path: String` | `ProcessResult`: structure items (functions, types, classes), imports, exports, symbols, metrics |
| `code.search` | `path: String`, `query: String`, `language?: String` | Matched spans with surrounding context, returned as a flat list with file path and line range |

`code.scan_repo` operates on the local filesystem path supplied by the agent. It walks the directory tree, skips binary files and files exceeding a configurable size limit (default 1 MiB), and returns a paginated result. Pagination is cursor-based to support large repos without buffering the entire listing.

`code.parse_file` shells out to tslp's `process()` pipeline with all extraction flags enabled. The returned `ProcessResult` is serialized to JSON and returned as the skill output.

`code.search` accepts a tree-sitter query string (S-expression syntax) and runs it against every parseable file under `path`. Results are streamed as they arrive; the MCP and gRPC transports surface this as a sequence of partial results.

### Skill input/output schemas

All skill inputs and outputs are JSON Schema objects, consistent with ADR-006. The `code.parse_file` output schema mirrors tslp's `ProcessResult` type, serialized with `serde` (tslp's `serde` feature enabled).

### Resource limits

To avoid runaway resource consumption on large repos:

- Max file size for parsing: 1 MiB (configurable via `[code_tools]` in `trumpet.toml`).
- Max files per `code.scan_repo` page: 500 (configurable).
- Parsing runs in a bounded `tokio::task::spawn_blocking` pool to avoid starving the async runtime.
- `code.search` short-circuits when match count exceeds a configurable cap (default 1 000 matches).

## Consequences

- `tree-sitter-language-pack` with the `serde` feature adds a significant compile-time cost due to the number of bundled C grammars. A first build will be slow; incremental builds are unaffected.
- All code analysis runs in the daemon process. Agents do not need tree-sitter as a dependency -- they send paths and receive structured JSON results.
- Language detection is best-effort. Files with ambiguous or unknown extensions fall back to content-based heuristics; unparsable files are returned with `language: null` and skipped during symbol extraction.
- The `code.search` skill exposes raw tree-sitter query syntax to agents. Malformed queries produce a structured error rather than panicking; the query string is validated before execution.
- This is the foundation for future skills: `code.rename_symbol`, `code.find_references`, `code.dependency_graph`. Those are out of scope for this ADR.
- The `[code_tools]` config section must be added to the config schema (ADR-010) to expose file size limits, page sizes, and match caps.
