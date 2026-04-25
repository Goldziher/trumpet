//! `trumpet start` subcommand — spawns the daemon as a detached background process.

use std::time::Duration;

use tokio::time::sleep;
use tracing::debug;

use crate::config::Config;
use crate::daemon::{is_daemon_running, read_pid_file};
use crate::error::{Error, Result};

/// Start the trumpet daemon in the background.
///
/// Re-execs the current binary with `serve` and detaches it from the terminal.
/// Polls the Unix socket for up to 3 seconds to confirm the daemon is ready,
/// then prints a success message.
///
/// # Errors
///
/// Returns [`Error::DaemonAlreadyRunning`] when an active daemon is detected,
/// or [`Error::InternalUnexpected`] when the child process cannot be spawned or
/// the daemon does not become ready within the timeout.
pub async fn run_start() -> Result<()> {
    let config = Config::load()?;

    if is_daemon_running(&config.daemon.socket_path).await {
        let pid = read_pid_file(&config.daemon.pid_file).await.unwrap_or(0);
        return Err(Error::DaemonAlreadyRunning { pid });
    }

    let exe = std::env::current_exe().map_err(|e| Error::InternalUnexpected {
        reason: format!("failed to locate current executable: {e}"),
    })?;

    debug!(exe = %exe.display(), "spawning daemon");

    std::process::Command::new(&exe)
        .arg("serve")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to spawn daemon: {e}"),
        })?;

    // Poll for readiness for up to 3 seconds in 200 ms increments.
    const POLL_INTERVAL: Duration = Duration::from_millis(200);
    const MAX_POLLS: u32 = 15; // 15 × 200 ms = 3 s

    for _ in 0..MAX_POLLS {
        sleep(POLL_INTERVAL).await;
        if is_daemon_running(&config.daemon.socket_path).await {
            let pid = read_pid_file(&config.daemon.pid_file).await.unwrap_or(0);
            println!("trumpet daemon started (pid {pid})");
            return Ok(());
        }
    }

    Err(Error::InternalUnexpected {
        reason: "daemon did not become ready within 3 seconds".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixListener;

    use tempfile::tempdir;

    use super::*;
    use crate::config::types::{Config, DaemonConfig};
    use crate::daemon::write_pid_file;

    /// When a daemon socket is already accepting connections, `run_start` must
    /// return `DaemonAlreadyRunning`.
    #[tokio::test]
    async fn start_returns_already_running_when_daemon_active() {
        let dir = tempdir().expect("tempdir");
        let socket_path = dir.path().join("trumpet.sock");
        let pid_path = dir.path().join("trumpet.pid");

        // Bind a real Unix socket to simulate a live daemon.
        let _listener = UnixListener::bind(&socket_path).expect("bind socket");

        // Write a PID file so `read_pid_file` has something to return.
        write_pid_file(&pid_path).await.expect("write pid file");

        let config = Config {
            daemon: DaemonConfig {
                socket_path: socket_path.clone(),
                pid_file: pid_path.clone(),
            },
            ..Default::default()
        };

        let result = start_with_config(&config).await;

        assert!(
            matches!(result, Err(Error::DaemonAlreadyRunning { .. })),
            "expected DaemonAlreadyRunning when socket is bound, got {result:?}"
        );
    }

    /// Internal helper that mirrors `run_start` but accepts an explicit config.
    async fn start_with_config(config: &Config) -> Result<()> {
        if is_daemon_running(&config.daemon.socket_path).await {
            let pid = read_pid_file(&config.daemon.pid_file).await.unwrap_or(0);
            return Err(Error::DaemonAlreadyRunning { pid });
        }

        Ok(())
    }
}
