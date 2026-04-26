//! `trumpet agent` subcommands — register, list, get, deregister, heartbeat.

use serde_json::json;

use crate::core::task_types::AgentCapabilities;
use crate::core::types::AgentInfo;
use crate::error::{Error, Result};

use super::client::{print_json_or, unix_get, unix_post};

/// Agent management subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum AgentCmd {
    /// Register a new agent with the daemon.
    Register {
        /// Human-readable agent name.
        #[arg(long)]
        name: String,
        /// Comma-separated skill tags (e.g. `review,lint`).
        #[arg(long, value_delimiter = ',')]
        tags: Vec<String>,
        /// Emit raw JSON instead of a human-readable summary.
        #[arg(long)]
        json: bool,
    },
    /// List all registered agents.
    List {
        /// Emit raw JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Look up an agent by ID.
    Get {
        /// Agent UUID.
        id: String,
        /// Emit raw JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
    /// Deregister an agent by ID.
    Deregister {
        /// Agent UUID.
        id: String,
    },
    /// Send a heartbeat for an agent.
    Heartbeat {
        /// Agent UUID.
        id: String,
    },
}

/// Dispatch an [`AgentCmd`] to the running daemon.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] when the daemon is unreachable.
pub async fn run(cmd: AgentCmd) -> Result<()> {
    match cmd {
        AgentCmd::Register { name, tags, json } => register(name, tags, json).await,
        AgentCmd::List { json } => list(json).await,
        AgentCmd::Get { id, json } => get(id, json).await,
        AgentCmd::Deregister { id } => deregister(id).await,
        AgentCmd::Heartbeat { id } => heartbeat(id).await,
    }
}

async fn register(name: String, tags: Vec<String>, json_flag: bool) -> Result<()> {
    let capabilities = if tags.is_empty() {
        None
    } else {
        Some(AgentCapabilities {
            skill_tags: tags,
            ..Default::default()
        })
    };

    let body = json!({
        "name": name,
        "capabilities": capabilities,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let (status, bytes) = unix_post("/agents/register", &body_bytes).await?;
    check_status(status, &bytes)?;

    let info: AgentInfo =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse agent response: {e}"),
        })?;

    print_json_or(&info, json_flag, |a| {
        println!("registered agent {} ({})", a.name, a.id);
    });
    Ok(())
}

async fn list(json_flag: bool) -> Result<()> {
    let (status, bytes) = unix_get("/agents").await?;
    check_status(status, &bytes)?;

    let agents: Vec<AgentInfo> =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse agents response: {e}"),
        })?;

    if json_flag {
        println!(
            "{}",
            serde_json::to_string(&agents).unwrap_or_else(|_| "[]".into())
        );
    } else {
        println!("{:<38}  {:<24}  {:<12}  TAGS", "ID", "NAME", "STATUS");
        for a in &agents {
            let tags = a
                .capabilities
                .as_ref()
                .map(|c| c.skill_tags.join(","))
                .unwrap_or_default();
            println!(
                "{:<38}  {:<24}  {:<12}  {}",
                a.id.to_string(),
                a.name,
                format!("{:?}", a.status).to_lowercase(),
                tags
            );
        }
    }
    Ok(())
}

async fn get(id: String, json_flag: bool) -> Result<()> {
    let (status, bytes) = unix_get("/agents").await?;
    check_status(status, &bytes)?;

    let agents: Vec<AgentInfo> =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse agents response: {e}"),
        })?;

    let Some(agent) = agents.into_iter().find(|a| a.id.to_string() == id) else {
        return Err(Error::AgentNotFound { name: id });
    };

    print_json_or(&agent, json_flag, |a| {
        println!("id:     {}", a.id);
        println!("name:   {}", a.name);
        println!("status: {:?}", a.status);
        if let Some(caps) = &a.capabilities {
            println!("tags:   {}", caps.skill_tags.join(", "));
        }
    });
    Ok(())
}

async fn deregister(id: String) -> Result<()> {
    let body = json!({ "agent_id": id });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let (status, bytes) = unix_post("/agents/deregister", &body_bytes).await?;
    check_status(status, &bytes)?;

    println!("deregistered agent {id}");
    Ok(())
}

async fn heartbeat(id: String) -> Result<()> {
    let path = format!("/agents/{id}/heartbeat");
    let (status, bytes) = unix_post(&path, b"{}").await?;
    check_status(status, &bytes)?;

    println!("heartbeat sent for agent {id}");
    Ok(())
}

/// Map a non-2xx HTTP status to an error.
pub(super) fn check_status(status: u16, body: &[u8]) -> Result<()> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(body).into_owned();
    Err(Error::InternalUnexpected {
        reason: format!("daemon returned HTTP {status}: {detail}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_status_accepts_2xx() {
        assert!(check_status(200, b"{}").is_ok(), "200 must be accepted");
        assert!(check_status(201, b"{}").is_ok(), "201 must be accepted");
    }

    #[test]
    fn check_status_rejects_4xx() {
        let result = check_status(404, b"not found");
        assert!(
            matches!(result, Err(Error::InternalUnexpected { .. })),
            "404 must produce InternalUnexpected"
        );
    }
}
