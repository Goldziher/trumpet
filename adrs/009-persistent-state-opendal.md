# ADR-009: Persistent State with OpenDAL

## Status

Accepted

## Context

The daemon (ADR-008) holds shared state: agent registrations, conversations, task boards, skill registry contents, and event history. This state lives in memory for performance, but must survive daemon restarts. We also want flexibility in where state is stored -- local disk for single-machine setups, object storage (S3, GCS) for distributed or cloud deployments.

## Decision

We will use [Apache OpenDAL](https://opendal.apache.org/) as our storage abstraction layer. OpenDAL provides a unified API across storage backends (fs, S3, GCS, Azure Blob, Redis, etc.) without locking us into any single one.

### Architecture

```text
┌─────────────────────────────────┐
│           In-Memory State       │
│  (agents, skills, conversations)│
└──────────┬──────────────────────┘
           │ write-behind / snapshot
┌──────────▼──────────────────────┐
│        State Manager            │
│  (serialization, journaling)    │
└──────────┬──────────────────────┘
           │ OpenDAL Operator
┌──────────▼──────────────────────┐
│     Storage Backend             │
│  fs | s3 | gcs | redis | ...   │
└─────────────────────────────────┘
```

### Strategy

- **In-memory first**: all reads hit memory. The in-memory state is the source of truth while the daemon is running.
- **Write-behind persistence**: state changes are batched and flushed to disk asynchronously at configurable intervals.
- **Snapshot on shutdown**: on graceful shutdown, a full state snapshot is written.
- **Restore on startup**: on startup, the daemon loads the latest snapshot and replays any journal entries.
- **Default backend**: `fs` backend storing state under `~/.trumpet/state/`. Configurable to any OpenDAL-supported backend.

### Serialization

State is serialized using a compact binary format (e.g. `bincode` or `MessagePack`) for snapshots, with an append-only journal for incremental changes between snapshots.

## Consequences

- OpenDAL gives us backend flexibility without writing storage adapters ourselves.
- In-memory-first means zero read latency for hot paths.
- Write-behind adds complexity: crash between flushes loses recent state. The journal mitigates this.
- OpenDAL is a large dependency with many optional features -- we should use feature flags to only compile the backends we need.
- `~/.trumpet/state/` becomes a well-known path that users need to be aware of for backups and debugging.
