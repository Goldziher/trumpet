# ADR-003: MCP Server Integration

## Status

Accepted

## Context

The Model Context Protocol (MCP) is an open standard for connecting AI models to external tools and data sources. By exposing the nexus as an MCP server, AI models (e.g. Claude, other LLM-based agents) can interact with the nexus natively -- discovering agents, submitting tasks, and reading results through the MCP tool/resource interface.

## Decision

We will implement an MCP server using the [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk). The MCP server will expose nexus capabilities as MCP tools and resources, allowing MCP-compatible clients to:

- List available agents and their capabilities.
- Submit tasks to agents.
- Read task results and agent state.

The MCP server runs as part of the axum server process (or as a standalone stdio transport for local development).

## Consequences

- Any MCP-compatible client can use the nexus without custom integration.
- We depend on the rust-sdk crate, which is relatively new -- we accept the risk of API churn.
- We need to design a clean mapping between nexus concepts (agents, tasks) and MCP concepts (tools, resources).
