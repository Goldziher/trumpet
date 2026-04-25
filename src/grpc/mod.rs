//! gRPC service implementation for the A2A (agent-to-agent) protocol.
//!
//! Uses the official A2A proto definition from `vendor/a2a/specification/a2a.proto`.

pub mod service;

/// Re-exported prost-generated types for the official A2A protocol (`lf.a2a.v1`).
pub mod proto {
    tonic::include_proto!("lf.a2a.v1");
}
