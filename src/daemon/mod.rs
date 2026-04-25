//! Daemon lifecycle utilities: PID files, socket cleanup, and readiness checks.

use std::path::Path;

use tokio::net::UnixStream;
use tracing::debug;

/// Ensure the trumpet data directory (e.g. `~/.trumpet/`) exists.
pub async fn ensure_trumpet_dir(path: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(path).await
}

/// Write the current process PID to a file with `0o600` permissions.
pub async fn write_pid_file(path: &Path) -> std::io::Result<()> {
    let pid = std::process::id();
    tokio::fs::write(path, pid.to_string()).await?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    }

    Ok(())
}

/// Read the PID from a PID file.
pub async fn read_pid_file(path: &Path) -> std::io::Result<u32> {
    let content = tokio::fs::read_to_string(path).await?;
    content
        .trim()
        .parse::<u32>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Remove a PID file if it exists, silently ignoring `NotFound`.
pub async fn remove_pid_file(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove a stale socket file if it exists, silently ignoring `NotFound`.
pub async fn remove_stale_socket(path: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {
            debug!("removed stale socket at {}", path.display());
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Check if the daemon is running by attempting to connect to its Unix socket.
pub async fn is_daemon_running(socket_path: &Path) -> bool {
    if !socket_path.exists() {
        return false;
    }
    UnixStream::connect(socket_path).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn ensure_trumpet_dir_creates_directory() {
        let base = tempdir().expect("tempdir");
        let target = base.path().join("nested").join("trumpet");

        ensure_trumpet_dir(&target)
            .await
            .expect("should create dirs");

        assert!(target.exists(), "directory should exist after creation");
        assert!(target.is_dir(), "path should be a directory");
    }

    #[tokio::test]
    async fn write_and_read_pid_file() {
        let dir = tempdir().expect("tempdir");
        let pid_path = dir.path().join("trumpet.pid");

        write_pid_file(&pid_path)
            .await
            .expect("should write pid file");

        let read_pid = read_pid_file(&pid_path)
            .await
            .expect("should read pid file");
        let current_pid = std::process::id();

        assert_eq!(
            read_pid, current_pid,
            "read PID must match current process PID"
        );
    }

    #[tokio::test]
    async fn remove_pid_file_nonexistent_succeeds() {
        let dir = tempdir().expect("tempdir");
        let pid_path = dir.path().join("nonexistent.pid");

        remove_pid_file(&pid_path)
            .await
            .expect("removing non-existent pid file should not error");
    }

    #[tokio::test]
    async fn remove_stale_socket_nonexistent_succeeds() {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join("nonexistent.sock");

        remove_stale_socket(&sock_path)
            .await
            .expect("removing non-existent socket should not error");
    }

    #[tokio::test]
    async fn is_daemon_running_returns_false_when_no_socket() {
        let dir = tempdir().expect("tempdir");
        let sock_path = dir.path().join("trumpet.sock");

        assert!(
            !is_daemon_running(&sock_path).await,
            "daemon should not be running when socket does not exist"
        );
    }
}
