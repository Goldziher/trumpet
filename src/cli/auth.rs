//! `trumpet auth` subcommands — show and rotate the daemon auth token.

use crate::config::Config;
use crate::error::{Error, Result};

/// Authentication management subcommands.
#[derive(clap::Subcommand, Debug)]
pub enum AuthCmd {
    /// Print the auth token path and its current value.
    Show,
    /// Generate a fresh token and signal the running daemon to reload.
    Rotate,
}

/// Dispatch an [`AuthCmd`].
///
/// # Errors
///
/// Returns an error when the token file cannot be read or the operation is
/// not yet implemented.
pub async fn run(cmd: AuthCmd) -> Result<()> {
    match cmd {
        AuthCmd::Show => show().await,
        AuthCmd::Rotate => Err(Error::InternalUnexpected {
            reason: "not yet wired — see F9".into(),
        }),
    }
}

async fn show() -> Result<()> {
    let config = Config::load().map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let path = &config.security.auth_token_path;
    let token = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("cannot read auth token at '{}': {e}", path.display()),
        })?;

    println!("path:  {}", path.display());
    println!("token: {}", token.trim());
    Ok(())
}
