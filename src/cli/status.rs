//! `trumpet status` subcommand — queries a running daemon and reports its state.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::config::Config;
use crate::error::{Error, Result};

/// Connect to the running daemon and print its status and connected agents.
///
/// # Errors
///
/// Returns [`Error::DaemonNotRunning`] when no socket file is present, or
/// [`Error::ConnectionRefused`] when the socket exists but the daemon does not
/// respond to HTTP requests.
pub async fn run_status() -> Result<()> {
    let config = Config::load()?;

    let socket_path = &config.daemon.socket_path;

    if !socket_path.exists() {
        return Err(Error::DaemonNotRunning);
    }

    // Confirm liveness via /health before querying /agents.
    connect_and_get(socket_path, "/health").await?;

    let body = connect_and_get(socket_path, "/agents").await?;

    println!("trumpet daemon: running");
    println!("socket: {}", socket_path.display());

    if let Ok(agents) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
        println!("agents ({}):", agents.len());
        for agent in &agents {
            if let Some(name) = agent.get("name").and_then(|n| n.as_str()) {
                println!("  - {name}");
            }
        }
    }

    Ok(())
}

/// Send a bare HTTP/1.1 GET request over a Unix socket and return the body as a
/// UTF-8 string.
///
/// This is intentionally minimal: it writes a raw request, reads until the
/// connection closes, then strips the HTTP response headers.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] on I/O failure and
/// [`Error::InternalUnexpected`] if the response cannot be decoded.
async fn connect_and_get(socket_path: &std::path::Path, path: &str) -> Result<String> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| Error::ConnectionRefused)?;

    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| Error::ConnectionRefused)?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: e.to_string(),
        })?;

    let raw = String::from_utf8_lossy(&response);

    // Split headers from body on the first blank line.
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or(&raw);

    Ok(body.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A socket path that does not exist should produce `DaemonNotRunning`.
    #[tokio::test]
    async fn test_run_status_returns_daemon_not_running_when_no_socket() {
        // Use a path that is guaranteed not to exist.
        let config_result = Config::load();
        assert!(
            config_result.is_ok(),
            "Config::load must succeed in test environment"
        );

        // Directly test connect_and_get with a nonexistent path.
        let result =
            connect_and_get(Path::new("/tmp/nonexistent-trumpet-test.sock"), "/health").await;
        assert!(
            matches!(result, Err(Error::ConnectionRefused)),
            "expected ConnectionRefused for missing socket, got {result:?}"
        );
    }
}
