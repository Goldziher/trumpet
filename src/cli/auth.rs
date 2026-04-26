//! `trumpet auth` subcommands — show and rotate the daemon auth token.

use std::path::Path;

use crate::config::Config;
use crate::daemon::read_pid_file;
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
/// Returns an error when the token file cannot be read or written, or when
/// the daemon's PID file cannot be located for SIGHUP delivery.
pub async fn run(cmd: AuthCmd) -> Result<()> {
    match cmd {
        AuthCmd::Show => show().await,
        AuthCmd::Rotate => rotate().await,
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

async fn rotate() -> Result<()> {
    let config = Config::load().map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let pid = read_pid_file(&config.daemon.pid_file)
        .await
        .map_err(|_| Error::DaemonNotRunning)?;

    let new_token = uuid::Uuid::new_v4().to_string();
    write_token_atomic(&config.security.auth_token_path, &new_token).await?;

    #[cfg(unix)]
    {
        let raw_pid = i32::try_from(pid).map_err(|_| Error::InternalUnexpected {
            reason: format!("PID {pid} exceeds i32::MAX"),
        })?;
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(raw_pid),
            nix::sys::signal::Signal::SIGHUP,
        )
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to send SIGHUP to pid {pid}: {e}"),
        })?;
    }
    #[cfg(not(unix))]
    {
        return Err(Error::InternalUnexpected {
            reason: "auth rotation requires SIGHUP, only supported on Unix".to_owned(),
        });
    }

    println!("rotated auth token; new value:");
    println!("{new_token}");
    Ok(())
}

/// Atomically replace the file at `path` with one whose content is `token`,
/// preserving owner-only `0o600` permissions.
async fn write_token_atomic(path: &Path, token: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::InternalUnexpected {
        reason: format!("auth token path has no parent: {}", path.display()),
    })?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("create parent dir {}: {e}", parent.display()),
        })?;

    let temp_path = parent.join(format!(".auth.token.rotate-{}", uuid::Uuid::new_v4()));
    write_token_with_owner_perms(&temp_path, token).await?;

    tokio::fs::rename(&temp_path, path)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("rename token over '{}': {e}", path.display()),
        })?;
    Ok(())
}

#[cfg(unix)]
async fn write_token_with_owner_perms(path: &Path, token: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let path = path.to_owned();
    let token = token.to_owned();
    tokio::task::spawn_blocking(move || -> Result<()> {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("create temp token file {}: {e}", path.display()),
            })?;
        file.write_all(token.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("write temp token file {}: {e}", path.display()),
            })?;
        Ok(())
    })
    .await
    .map_err(|e| Error::InternalUnexpected {
        reason: format!("blocking write task panicked: {e}"),
    })?
}

#[cfg(not(unix))]
async fn write_token_with_owner_perms(path: &Path, token: &str) -> Result<()> {
    tokio::fs::write(path, token)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("write token: {e}"),
        })
}
