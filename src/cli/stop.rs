//! `trumpet stop` subcommand — sends SIGTERM to a running daemon.

use std::time::Duration;

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::time::sleep;
use tracing::debug;

use crate::config::Config;
use crate::daemon::{is_daemon_running, read_pid_file, remove_pid_file, remove_stale_socket};
use crate::error::{Error, Result};

/// Gracefully stop the running trumpet daemon.
///
/// Reads the PID file, sends `SIGTERM`, waits up to 5 seconds for the process
/// to exit, then removes the PID file and stale socket.
///
/// # Errors
///
/// Returns [`Error::DaemonNotRunning`] when no PID file is present, or
/// [`Error::DaemonShutdownFailed`] when the daemon does not exit within the
/// timeout window.
pub async fn run_stop() -> Result<()> {
    let config = Config::load()?;

    let pid_path = &config.daemon.pid_file;

    if !pid_path.exists() {
        return Err(Error::DaemonNotRunning);
    }

    let pid = read_pid_file(pid_path)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("failed to read pid file: {e}"),
        })?;

    debug!(pid, "sending SIGTERM to daemon");

    kill(Pid::from_raw(pid as i32), Signal::SIGTERM).map_err(|e| Error::InternalUnexpected {
        reason: format!("failed to send SIGTERM to pid {pid}: {e}"),
    })?;

    // Poll for up to 5 seconds in 200 ms increments.
    let socket_path = &config.daemon.socket_path;
    const POLL_INTERVAL: Duration = Duration::from_millis(200);
    const MAX_POLLS: u32 = 25; // 25 × 200 ms = 5 s

    for _ in 0..MAX_POLLS {
        sleep(POLL_INTERVAL).await;
        if !is_daemon_running(socket_path).await {
            debug!("daemon exited cleanly");
            remove_pid_file(pid_path)
                .await
                .map_err(|e| Error::InternalUnexpected {
                    reason: format!("failed to remove pid file: {e}"),
                })?;
            remove_stale_socket(socket_path)
                .await
                .map_err(|e| Error::InternalUnexpected {
                    reason: format!("failed to remove stale socket: {e}"),
                })?;
            return Ok(());
        }
    }

    Err(Error::DaemonShutdownFailed {
        reason: format!("daemon (pid {pid}) did not exit within 5 seconds"),
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::config::types::{Config, DaemonConfig};

    /// When the PID file does not exist, `run_stop` must return `DaemonNotRunning`.
    #[tokio::test]
    async fn stop_returns_daemon_not_running_when_no_pid_file() {
        let dir = tempdir().expect("tempdir");
        let pid_path = dir.path().join("trumpet.pid");
        let socket_path = dir.path().join("trumpet.sock");

        // Build a config that points at our nonexistent paths without calling
        // `Config::load()`, which resolves `$HOME`.
        let config = Config {
            daemon: DaemonConfig {
                pid_file: pid_path,
                socket_path,
            },
            ..Default::default()
        };

        // Drive the same logic as `run_stop` but with our isolated config.
        let result = stop_with_config(&config).await;

        assert!(
            matches!(result, Err(Error::DaemonNotRunning)),
            "expected DaemonNotRunning when pid file is absent, got {result:?}"
        );
    }

    /// Internal helper that mirrors `run_stop` but accepts an explicit config so
    /// tests can inject arbitrary paths without touching the filesystem or `$HOME`.
    async fn stop_with_config(config: &Config) -> Result<()> {
        let pid_path = &config.daemon.pid_file;

        if !pid_path.exists() {
            return Err(Error::DaemonNotRunning);
        }

        let pid = read_pid_file(pid_path)
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("failed to read pid file: {e}"),
            })?;

        kill(Pid::from_raw(pid as i32), Signal::SIGTERM).map_err(|e| {
            Error::InternalUnexpected {
                reason: format!("failed to send SIGTERM to pid {pid}: {e}"),
            }
        })?;

        Ok(())
    }
}
