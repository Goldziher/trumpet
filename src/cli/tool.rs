//! `trumpet tool` subcommands — list and invoke registered tools.

use crate::core::types::ToolInfo;
use crate::error::{Error, Result};

use super::agent::check_status;
use super::client::{unix_get, unix_post};

/// Tool management subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum ToolCmd {
    /// List all registered tools.
    List {
        /// Emit raw JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Invoke a tool by name.
    Invoke {
        /// Tool name (e.g. `code.scan_repo`).
        name: String,
        /// JSON-encoded input payload.
        #[arg(long)]
        input: String,
    },
}

/// Dispatch a [`ToolCmd`] to the running daemon.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] when the daemon is unreachable.
pub async fn run(cmd: ToolCmd) -> Result<()> {
    match cmd {
        ToolCmd::List { json } => list(json).await,
        ToolCmd::Invoke { name, input } => invoke(name, input).await,
    }
}

async fn list(json_flag: bool) -> Result<()> {
    let (status, bytes) = unix_get("/tools").await?;
    check_status(status, &bytes)?;

    let tools: Vec<ToolInfo> =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse tools response: {e}"),
        })?;

    if json_flag {
        println!(
            "{}",
            serde_json::to_string(&tools).unwrap_or_else(|_| "[]".into())
        );
    } else {
        println!("{:<32}  DESCRIPTION", "NAME");
        for t in &tools {
            println!("{:<32}  {}", t.name, t.description);
        }
    }
    Ok(())
}

async fn invoke(name: String, input: String) -> Result<()> {
    let input_value: serde_json::Value =
        serde_json::from_str(&input).map_err(|e| Error::InvalidInput {
            reason: format!("--input is not valid JSON: {e}"),
        })?;

    let body = serde_json::json!({ "input": input_value });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let path = format!("/tools/{name}/invoke");
    let (status, bytes) = unix_post(&path, &body_bytes).await?;
    check_status(status, &bytes)?;

    let result: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse tool response: {e}"),
        })?;

    println!(
        "{}",
        serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into())
    );
    Ok(())
}
