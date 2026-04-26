//! `trumpet task` subcommands — submit, list, get, cancel, watch.

use serde_json::json;

use crate::core::task_types::{Task, TaskState};
use crate::error::{Error, Result};

use super::agent::check_status;
use super::client::{print_json_or, unix_get, unix_post};

/// Task management subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum TaskCmd {
    /// Submit a new task to the daemon.
    Submit {
        /// Task message text.
        #[arg(long)]
        message: String,
        /// Optional context (session) UUID.
        #[arg(long)]
        context_id: Option<String>,
        /// Optional assignee agent UUID.
        #[arg(long)]
        assignee: Option<String>,
        /// Optional deadline in milliseconds from now.
        #[arg(long)]
        deadline_ms: Option<u64>,
        /// Emit raw JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
    /// List tasks, optionally filtered.
    List {
        /// Filter by task state (e.g. `submitted`, `working`, `completed`).
        #[arg(long)]
        state: Option<String>,
        /// Filter by assignee agent UUID.
        #[arg(long)]
        assignee: Option<String>,
        /// Filter by context UUID.
        #[arg(long)]
        context_id: Option<String>,
        /// Emit raw JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Get a single task by ID.
    Get {
        /// Task UUID.
        id: String,
        /// Emit raw JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
    /// Cancel a task.
    Cancel {
        /// Task UUID.
        id: String,
        /// Optional cancellation reason.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Stream events for a task until it reaches a terminal state.
    Watch {
        /// Task UUID.
        id: String,
    },
}

/// Dispatch a [`TaskCmd`] to the running daemon.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] when the daemon is unreachable.
pub async fn run(cmd: TaskCmd) -> Result<()> {
    match cmd {
        TaskCmd::Submit {
            message,
            context_id,
            assignee,
            deadline_ms,
            json,
        } => submit(message, context_id, assignee, deadline_ms, json).await,
        TaskCmd::List {
            state,
            assignee,
            context_id,
            json,
        } => list(state, assignee, context_id, json).await,
        TaskCmd::Get { id, json } => get(id, json).await,
        TaskCmd::Cancel { id, reason } => cancel(id, reason).await,
        TaskCmd::Watch { id } => watch(id).await,
    }
}

async fn submit(
    message: String,
    context_id: Option<String>,
    assignee: Option<String>,
    deadline_ms: Option<u64>,
    json_flag: bool,
) -> Result<()> {
    let body = json!({
        "message": message,
        "context_id": context_id,
        "assignee": assignee,
        "deadline_ms": deadline_ms,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let (status, bytes) = unix_post("/tasks", &body_bytes).await?;
    check_status(status, &bytes)?;

    let task: Task = serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
        reason: format!("failed to parse task response: {e}"),
    })?;

    print_json_or(&task, json_flag, |t| {
        println!("submitted task {}", t.id);
    });
    Ok(())
}

async fn list(
    state: Option<String>,
    assignee: Option<String>,
    context_id: Option<String>,
    json_flag: bool,
) -> Result<()> {
    let mut params: Vec<String> = Vec::new();
    if let Some(s) = state {
        params.push(format!("state={s}"));
    }
    if let Some(a) = assignee {
        params.push(format!("assignee={a}"));
    }
    if let Some(c) = context_id {
        params.push(format!("context_id={c}"));
    }

    let path = if params.is_empty() {
        "/tasks".to_owned()
    } else {
        format!("/tasks?{}", params.join("&"))
    };

    let (status, bytes) = unix_get(&path).await?;
    check_status(status, &bytes)?;

    let tasks: Vec<Task> =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse tasks response: {e}"),
        })?;

    if json_flag {
        println!(
            "{}",
            serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".into())
        );
    } else {
        println!("{:<38}  {:<14}  ASSIGNEE", "ID", "STATE");
        for t in &tasks {
            let assignee = t
                .assignee
                .as_ref()
                .map(|a| a.to_string())
                .unwrap_or_default();
            println!("{:<38}  {:<14}  {}", t.id, t.status.state, assignee);
        }
    }
    Ok(())
}

async fn get(id: String, json_flag: bool) -> Result<()> {
    let path = format!("/tasks/{id}");
    let (status, bytes) = unix_get(&path).await?;
    check_status(status, &bytes)?;

    let task: Task = serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
        reason: format!("failed to parse task response: {e}"),
    })?;

    print_json_or(&task, json_flag, |t| {
        println!("id:       {}", t.id);
        println!("state:    {}", t.status.state);
        if let Some(a) = &t.assignee {
            println!("assignee: {a}");
        }
        println!("context:  {}", t.context_id);
    });
    Ok(())
}

async fn cancel(id: String, _reason: Option<String>) -> Result<()> {
    let path = format!("/tasks/{id}/cancel");
    let (status, bytes) = unix_post(&path, b"{}").await?;
    check_status(status, &bytes)?;

    println!("canceled task {id}");
    Ok(())
}

async fn watch(id: String) -> Result<()> {
    use super::client::unix_get_streaming;
    use super::events::SseLineParser;

    println!("watching task {id} (ctrl-c to stop)");

    let mut parser = SseLineParser::default();
    let mut terminal = false;

    unix_get_streaming("/events", move |chunk| {
        if terminal {
            return;
        }
        for frame in parser.feed(chunk) {
            let Some(data) = frame.data.as_deref() else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
                continue;
            };
            let task_match = v
                .get("task_id")
                .and_then(|x| x.as_str())
                .map(|x| x == id)
                .unwrap_or(false);
            if !task_match {
                continue;
            }
            let event_type = v.get("type").and_then(|x| x.as_str()).unwrap_or("unknown");
            println!("[{event_type}] {data}");

            if let Some(state_str) = v.get("new_state").and_then(|x| x.as_str())
                && let Ok(state) = state_str.parse::<TaskState>()
                && state.is_terminal()
            {
                terminal = true;
            }
        }
    })
    .await
}
