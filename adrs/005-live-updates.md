# ADR-005: Live Updates for Connected Agents

## Status

Accepted

## Context

Agents connected to the nexus need to receive real-time updates -- new tasks assigned, skill availability changes, inter-agent messages, and state transitions. Polling is wasteful and introduces latency. We need push-based notification across all protocol interfaces.

## Decision

We will support live updates through the following mechanisms per transport:

- **MCP**: Use the built-in notification system. The server emits `notifications/tools/list_changed` when skills change and `notifications/resources/updated` when subscribed resources change. Clients use `resources/subscribe` / `resources/unsubscribe` to manage subscriptions.
- **gRPC (A2A)**: Define streaming RPCs in `a2a.proto`. Agents open a bidirectional stream on connection and receive events as they occur (task assignments, messages, skill changes).
- **WebSocket (axum)**: Expose a `/ws` endpoint. Agents connect and receive a JSON event stream. Supports topic-based filtering.

Internally, the core uses a **broadcast channel** (e.g. `tokio::sync::broadcast`) to fan out events. Each transport adapter subscribes to this channel and forwards events in its native format.

## Consequences

- All three transports support push, so agents can use whichever protocol they prefer.
- The internal broadcast channel decouples the core from transport specifics.
- We need to handle backpressure -- slow consumers on WebSocket or gRPC streams must not block the broadcast channel.
- MCP notifications are fire-and-forget (no ack), so we cannot guarantee delivery over MCP. For critical updates, agents should use gRPC or WebSocket with acknowledgment.
