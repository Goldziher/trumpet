//! `trumpet chat` subcommands — new, list, send, history.

use serde_json::json;

use crate::core::types::{ChatMessage, Conversation};
use crate::error::{Error, Result};

use super::agent::check_status;
use super::client::{print_json_or, unix_get, unix_post};

/// Conversation management subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum ChatCmd {
    /// Create a new conversation.
    New {
        /// Optional display name for the conversation.
        #[arg(long)]
        name: Option<String>,
        /// Comma-separated participant agent UUIDs.
        #[arg(long, value_delimiter = ',')]
        participants: Vec<String>,
    },
    /// List all conversations.
    List {
        /// Emit raw JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Send a message into a conversation.
    Send {
        /// Conversation UUID.
        conversation_id: String,
        /// Sender agent UUID.
        #[arg(long, name = "as")]
        sender: String,
        /// Message text.
        #[arg(long)]
        content: String,
    },
    /// Print all messages in a conversation in chronological order.
    History {
        /// Conversation UUID.
        conversation_id: String,
        /// Emit raw JSON instead of formatted output.
        #[arg(long)]
        json: bool,
    },
}

/// Dispatch a [`ChatCmd`] to the running daemon.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] when the daemon is unreachable.
pub async fn run(cmd: ChatCmd) -> Result<()> {
    match cmd {
        ChatCmd::New { name, participants } => new(name, participants).await,
        ChatCmd::List { json } => list(json).await,
        ChatCmd::Send {
            conversation_id,
            sender,
            content,
        } => send(conversation_id, sender, content).await,
        ChatCmd::History {
            conversation_id,
            json,
        } => history(conversation_id, json).await,
    }
}

async fn new(name: Option<String>, participants: Vec<String>) -> Result<()> {
    let body = json!({
        "name": name,
        "participants": participants,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let (status, bytes) = unix_post("/conversations", &body_bytes).await?;
    check_status(status, &bytes)?;

    let conv: Conversation =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse conversation response: {e}"),
        })?;

    println!("created conversation {}", conv.id);
    Ok(())
}

async fn list(json_flag: bool) -> Result<()> {
    let (status, bytes) = unix_get("/conversations").await?;
    check_status(status, &bytes)?;

    let convs: Vec<Conversation> =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse conversations response: {e}"),
        })?;

    if json_flag {
        println!(
            "{}",
            serde_json::to_string(&convs).unwrap_or_else(|_| "[]".into())
        );
    } else {
        println!("{:<38}  {:<24}  PARTICIPANTS", "ID", "NAME");
        for c in &convs {
            let name = c.name.as_deref().unwrap_or("-");
            let participants = c.participants.len();
            println!("{:<38}  {:<24}  {participants}", c.id, name);
        }
    }
    Ok(())
}

async fn send(conversation_id: String, sender: String, content: String) -> Result<()> {
    let body = json!({
        "sender": sender,
        "content": content,
    });
    let body_bytes = serde_json::to_vec(&body).map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let path = format!("/conversations/{conversation_id}/messages");
    let (status, bytes) = unix_post(&path, &body_bytes).await?;
    check_status(status, &bytes)?;

    let msg: ChatMessage =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse message response: {e}"),
        })?;

    println!("sent message {}", msg.id);
    Ok(())
}

async fn history(conversation_id: String, json_flag: bool) -> Result<()> {
    let path = format!("/conversations/{conversation_id}/messages");
    let (status, bytes) = unix_get(&path).await?;
    check_status(status, &bytes)?;

    let messages: Vec<ChatMessage> =
        serde_json::from_slice(&bytes).map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to parse messages response: {e}"),
        })?;

    print_json_or(&messages, json_flag, |msgs| {
        for m in msgs {
            println!(
                "[{}] {}: {}",
                m.timestamp.format("%H:%M:%S"),
                m.sender,
                m.content
            );
        }
    });
    Ok(())
}
