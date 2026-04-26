//! CLI subcommand implementations.
//!
//! Each subcommand lives in its own module. This module re-exports the async
//! entry points consumed by `main.rs`.

pub mod agent;
pub mod auth;
pub mod chat;
pub mod client;
pub mod events;
mod serve;
mod start;
mod status;
mod stop;
pub mod task;
pub mod tool;

pub use agent::AgentCmd;
pub use auth::AuthCmd;
pub use chat::ChatCmd;
pub use events::EventsTailArgs;
pub use serve::run_serve;
pub use start::run_start;
pub use status::run_status;
pub use stop::run_stop;
pub use task::TaskCmd;
pub use tool::ToolCmd;
