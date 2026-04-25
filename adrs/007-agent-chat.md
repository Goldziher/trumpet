# ADR-007: Chat-Based Inter-Agent Communication

## Status

Accepted

## Context

Agents working on complex tasks need to collaborate in a conversational, multi-turn fashion -- not just fire-and-forget messages. For example, one agent might ask another to review code, receive feedback, refine, and iterate. This resembles a chat thread more than an RPC call.

## Decision

We will implement a **conversation manager** in the core that supports chat-like message exchanges between agents. Key concepts:

- **Conversation**: a named, ordered sequence of messages between two or more agents. Has a topic/purpose and a list of participants.
- **Message**: a timestamped payload from one agent to a conversation. Supports text, structured data (JSON), and references to skills or tasks.
- **Subscription**: agents subscribe to conversations they participate in and receive new messages as live updates (via the broadcast channel from ADR-005).

### Flow

1. Agent A creates a conversation with Agent B (and optionally others).
2. Agent A sends a message to the conversation.
3. Agent B receives the message as a live update (push notification).
4. Agent B responds. Agent A receives the response.
5. Either agent can close the conversation or add participants.

### Protocol mapping

- **gRPC**: bidirectional streaming RPC -- agents open a stream per conversation and exchange messages in real time.
- **MCP**: conversations are exposed as resources. Agents subscribe via `resources/subscribe` and receive `notifications/resources/updated` on new messages. Sending a message is an MCP tool call.
- **WebSocket**: JSON messages on the `/ws` endpoint with conversation routing.

## Consequences

- Enables natural, collaborative workflows between agents (review cycles, negotiation, planning).
- Conversation state must be stored (at least in memory, optionally persisted) -- adds state management complexity.
- We need to handle agent disconnects gracefully -- buffer messages and deliver on reconnect, or mark as undeliverable.
- Conversation history serves as an audit trail for multi-agent decision making.
