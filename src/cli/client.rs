//! Shared helpers for CLI subcommands that talk to the running daemon over
//! its Unix socket.
//!
//! All helpers load [`Config`] to find the socket path, then issue raw
//! HTTP/1.1 requests over the socket — no HTTP library needed, matching the
//! pattern in [`crate::cli::status`].

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::config::Config;
use crate::error::{Error, Result};

/// Send a bare HTTP GET over the daemon Unix socket.
///
/// Returns `(status_code, body_bytes)`.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] on I/O failure and
/// [`Error::InternalUnexpected`] if the response cannot be read.
pub async fn unix_get(path: &str) -> Result<(u16, Vec<u8>)> {
    let config = Config::load().map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;
    unix_get_on(&config.daemon.socket_path, path).await
}

/// Send a bare HTTP GET over an explicit socket path.
pub(crate) async fn unix_get_on(
    socket_path: &std::path::Path,
    path: &str,
) -> Result<(u16, Vec<u8>)> {
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

    split_response(response)
}

/// Send a bare HTTP POST with a JSON body over the daemon Unix socket.
///
/// Returns `(status_code, body_bytes)`.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] on I/O failure.
pub async fn unix_post(path: &str, body: &[u8]) -> Result<(u16, Vec<u8>)> {
    let config = Config::load().map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;
    unix_post_on(&config.daemon.socket_path, path, body).await
}

/// Send a bare HTTP POST over an explicit socket path.
pub(crate) async fn unix_post_on(
    socket_path: &std::path::Path,
    path: &str,
    body: &[u8],
) -> Result<(u16, Vec<u8>)> {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .map_err(|_| Error::ConnectionRefused)?;

    let header = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|_| Error::ConnectionRefused)?;
    stream
        .write_all(body)
        .await
        .map_err(|_| Error::ConnectionRefused)?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: e.to_string(),
        })?;

    split_response(response)
}

/// Open a streaming HTTP GET over the daemon Unix socket and call `on_chunk`
/// for each chunk of bytes received.
///
/// This is used by `trumpet events tail` and `trumpet task watch` for SSE
/// streams. The connection stays open until the daemon closes it or the
/// process is killed.
///
/// # Errors
///
/// Returns [`Error::ConnectionRefused`] on I/O failure.
pub async fn unix_get_streaming(path: &str, mut on_chunk: impl FnMut(&[u8])) -> Result<()> {
    let config = Config::load().map_err(|e| Error::InternalUnexpected {
        reason: e.to_string(),
    })?;

    let mut stream = UnixStream::connect(&config.daemon.socket_path)
        .await
        .map_err(|_| Error::ConnectionRefused)?;

    let request =
        format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|_| Error::ConnectionRefused)?;

    let mut buf = vec![0u8; 4096];
    let mut header_consumed = false;
    let mut leftover = Vec::new();

    loop {
        let n = stream
            .read(&mut buf)
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: e.to_string(),
            })?;
        if n == 0 {
            break;
        }

        if !header_consumed {
            leftover.extend_from_slice(&buf[..n]);
            if let Some(idx) = find_header_end(&leftover) {
                let body_start = idx + 4;
                let body = leftover[body_start..].to_vec();
                header_consumed = true;
                leftover = Vec::new();
                if !body.is_empty() {
                    on_chunk(&body);
                }
            }
        } else {
            on_chunk(&buf[..n]);
        }
    }

    Ok(())
}

/// Print `value` as compact JSON when `json_flag` is true; otherwise call
/// `fallback`.
pub fn print_json_or<T: Serialize>(value: &T, json_flag: bool, fallback: impl FnOnce(&T)) {
    if json_flag {
        println!(
            "{}",
            serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
        );
    } else {
        fallback(value);
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Split a raw HTTP response into `(status_code, body_bytes)`.
fn split_response(raw: Vec<u8>) -> Result<(u16, Vec<u8>)> {
    let status = parse_status_code(&raw).unwrap_or(200);

    let body = if let Some(idx) = find_header_end(&raw) {
        raw[idx + 4..].to_vec()
    } else {
        raw
    };

    Ok((status, body))
}

/// Find the byte offset of `\r\n\r\n` in `data`.
fn find_header_end(data: &[u8]) -> Option<usize> {
    data.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Parse the HTTP status code from the first line of a response.
fn parse_status_code(data: &[u8]) -> Option<u16> {
    let line_end = data.windows(2).position(|w| w == b"\r\n")?;
    let line = std::str::from_utf8(&data[..line_end]).ok()?;
    let mut parts = line.splitn(3, ' ');
    parts.next()?;
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_code_extracts_200() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}";
        assert_eq!(
            parse_status_code(raw),
            Some(200),
            "must extract 200 from a 200 OK response"
        );
    }

    #[test]
    fn parse_status_code_extracts_404() {
        let raw = b"HTTP/1.1 404 Not Found\r\n\r\n";
        assert_eq!(
            parse_status_code(raw),
            Some(404),
            "must extract 404 from a 404 response"
        );
    }

    #[test]
    fn find_header_end_locates_separator() {
        let raw = b"HTTP/1.1 200 OK\r\n\r\nbody";
        assert_eq!(
            find_header_end(raw),
            Some(15),
            "must find the header/body separator"
        );
    }

    #[test]
    fn split_response_separates_status_and_body() {
        let raw = b"HTTP/1.1 201 Created\r\n\r\n{\"id\":\"abc\"}".to_vec();
        let (status, body) = split_response(raw).expect("split must succeed");
        assert_eq!(status, 201, "status must be 201");
        assert_eq!(body, b"{\"id\":\"abc\"}", "body must match");
    }
}
