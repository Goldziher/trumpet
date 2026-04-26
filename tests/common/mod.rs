//! Shared scaffolding for end-to-end integration tests.
//!
//! [`TestDaemon::spawn`] launches an in-process Trumpet daemon against an
//! ephemeral Unix socket and TCP port under a fresh tempdir. Drop = clean
//! shutdown (cancel + abort + tempdir teardown).

#![allow(dead_code)]

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use trumpet::config::types::{
    AgentsConfig, CodeToolsConfig, Config, DaemonConfig, LogFormat, LoggingConfig, McpConfig,
    McpTransport, SecurityConfig, ServerConfig, StorageConfig,
};

/// Handle to a spawned daemon plus the ambient state needed to talk to it.
pub struct TestDaemon {
    pub socket_path: PathBuf,
    pub grpc_addr: String,
    pub grpc_port: u16,
    pub auth_token: String,
    pub config: Arc<Config>,
    pub temp_dir: PathBuf,
    cancel: CancellationToken,
    handle: Option<JoinHandle<trumpet::error::Result<()>>>,
    _temp: TempDir,
}

impl TestDaemon {
    /// Default builder; spawns with auth on, MCP off, snapshots disabled.
    pub async fn spawn() -> Self {
        TestDaemonBuilder::default().spawn().await
    }

    pub fn builder() -> TestDaemonBuilder {
        TestDaemonBuilder::default()
    }

    /// Cancel the daemon and await graceful shutdown.
    ///
    /// Returns the daemon's exit `Result`. After this call any later
    /// interactions over the socket / gRPC port will fail.
    pub async fn shutdown(mut self) -> trumpet::error::Result<()> {
        self.cancel.cancel();
        if let Some(h) = self.handle.take() {
            match tokio::time::timeout(Duration::from_secs(5), h).await {
                Ok(Ok(res)) => res,
                Ok(Err(e)) => Err(trumpet::error::Error::InternalUnexpected {
                    reason: format!("daemon task panicked: {e}"),
                }),
                Err(_) => Err(trumpet::error::Error::InternalUnexpected {
                    reason: "daemon did not stop within 5s".into(),
                }),
            }
        } else {
            Ok(())
        }
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Builder for [`TestDaemon`]. All fields default to values that make tests
/// fast and predictable: auth on, MCP off, snapshots every hour.
#[derive(Default)]
pub struct TestDaemonBuilder {
    require_auth: Option<bool>,
    mcp_enabled: bool,
    mcp_transport: Option<McpTransport>,
    snapshot_interval_secs: Option<u64>,
    storage_path: Option<PathBuf>,
    auth_token_path: Option<PathBuf>,
    heartbeat_interval_secs: Option<u64>,
    agent_timeout_secs: Option<u64>,
}

impl TestDaemonBuilder {
    pub fn require_auth(mut self, value: bool) -> Self {
        self.require_auth = Some(value);
        self
    }

    pub fn mcp_enabled(mut self, value: bool) -> Self {
        self.mcp_enabled = value;
        self
    }

    pub fn mcp_transport(mut self, value: McpTransport) -> Self {
        self.mcp_transport = Some(value);
        self
    }

    pub fn snapshot_interval_secs(mut self, value: u64) -> Self {
        self.snapshot_interval_secs = Some(value);
        self
    }

    /// Reuse a storage path from a prior daemon to test snapshot restore.
    pub fn storage_path(mut self, value: PathBuf) -> Self {
        self.storage_path = Some(value);
        self
    }

    /// Reuse an auth token path so a second daemon picks up the same token.
    pub fn auth_token_path(mut self, value: PathBuf) -> Self {
        self.auth_token_path = Some(value);
        self
    }

    pub fn heartbeat_interval_secs(mut self, value: u64) -> Self {
        self.heartbeat_interval_secs = Some(value);
        self
    }

    pub fn agent_timeout_secs(mut self, value: u64) -> Self {
        self.agent_timeout_secs = Some(value);
        self
    }

    pub async fn spawn(self) -> TestDaemon {
        let temp = TempDir::new().expect("create tempdir");
        let trumpet_dir = temp.path().join("trumpet");
        std::fs::create_dir_all(&trumpet_dir).expect("create trumpet dir");

        let socket_path = trumpet_dir.join("trumpet.sock");
        let grpc_port = pick_unused_port();
        let storage_path = self
            .storage_path
            .unwrap_or_else(|| trumpet_dir.join("state"));
        let auth_token_path = self
            .auth_token_path
            .unwrap_or_else(|| trumpet_dir.join("auth.token"));

        let mut agents = AgentsConfig::default();
        if let Some(v) = self.heartbeat_interval_secs {
            agents.heartbeat_interval_secs = v;
        }
        if let Some(v) = self.agent_timeout_secs {
            agents.timeout_secs = v;
        }

        let config = Config {
            daemon: DaemonConfig {
                socket_path: socket_path.clone(),
                pid_file: trumpet_dir.join("trumpet.pid"),
            },
            server: ServerConfig {
                host: "127.0.0.1".into(),
                http_port: 0,
                grpc_port,
            },
            mcp: McpConfig {
                enabled: self.mcp_enabled,
                transport: self.mcp_transport.unwrap_or(McpTransport::Stdio),
            },
            storage: StorageConfig {
                backend: "fs".into(),
                path: storage_path,
                snapshot_interval_secs: self.snapshot_interval_secs.unwrap_or(3600),
            },
            logging: LoggingConfig {
                level: "warn".into(),
                format: LogFormat::Text,
            },
            agents,
            code_tools: CodeToolsConfig {
                workspace_root: Some(trumpet_dir.clone()),
                ..Default::default()
            },
            security: SecurityConfig {
                auth_token_path: auth_token_path.clone(),
                require_auth: self.require_auth.unwrap_or(true),
            },
        };

        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();
        let cfg = config.clone();
        let handle =
            tokio::spawn(
                async move { trumpet::server::serve_with_shutdown(&cfg, cancel_clone).await },
            );

        wait_for_socket(&socket_path, Duration::from_secs(10)).await;
        wait_for_tcp(&format!("127.0.0.1:{grpc_port}"), Duration::from_secs(10)).await;

        let auth_token = std::fs::read_to_string(&auth_token_path)
            .expect("auth token file must exist after startup")
            .trim()
            .to_owned();

        TestDaemon {
            socket_path,
            grpc_addr: format!("127.0.0.1:{grpc_port}"),
            grpc_port,
            auth_token,
            config: Arc::new(config),
            temp_dir: trumpet_dir,
            cancel,
            handle: Some(handle),
            _temp: temp,
        }
    }
}

/// Pick an unused TCP port by binding `127.0.0.1:0` and immediately dropping.
///
/// Standard "find a free port" technique. There is a small race window where
/// another process could grab the port before we re-bind, but that's
/// acceptable for tests on a developer machine.
fn pick_unused_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn wait_for_socket(path: &std::path::Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if path.exists()
            && let Ok(stream) = tokio::net::UnixStream::connect(path).await
        {
            drop(stream);
            return;
        }
        if Instant::now() > deadline {
            panic!(
                "daemon Unix socket {} did not become connectable within {:?}",
                path.display(),
                timeout
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_tcp(addr: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        if Instant::now() > deadline {
            panic!("gRPC port {addr} did not accept connections within {timeout:?}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Send an HTTP/1.1 GET over the test daemon's Unix socket and return the
/// response status + body.
///
/// We talk to the socket directly rather than through `reqwest` so the test
/// scaffolding has no dependency on hyper-util's UnixConnector — the format
/// the daemon serves is plain HTTP/1.1 keep-alive.
pub async fn unix_get(socket: &std::path::Path, path: &str) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to daemon socket");
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut buf = Vec::with_capacity(4096);
    stream.read_to_end(&mut buf).await.expect("read");

    parse_http_response(&buf)
}

/// Send an HTTP/1.1 POST over the daemon's Unix socket. `body` is sent
/// verbatim (caller is responsible for setting the right `Content-Type`).
pub async fn unix_post(
    socket: &std::path::Path,
    path: &str,
    content_type: &str,
    body: &[u8],
) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to daemon socket");
    let header = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .await
        .expect("write header");
    stream.write_all(body).await.expect("write body");
    let mut buf = Vec::with_capacity(4096);
    stream.read_to_end(&mut buf).await.expect("read");

    parse_http_response(&buf)
}

fn parse_http_response(raw: &[u8]) -> (u16, Vec<u8>) {
    let (status, _headers, body) = parse_http_response_full(raw);
    (status, body)
}

/// Send an arbitrary HTTP/1.1 request over the daemon's Unix socket.
///
/// Generalises [`unix_get`] / [`unix_post`] for tests that need to set
/// custom request headers (e.g. MCP requires `Accept: application/json,
/// text/event-stream`) or read response headers (e.g. `Mcp-Session-Id`).
pub async fn unix_request_with_headers(
    socket: &std::path::Path,
    method: &str,
    path: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::UnixStream::connect(socket)
        .await
        .expect("connect to daemon socket");
    let mut header = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in extra_headers {
        header.push_str(&format!("{k}: {v}\r\n"));
    }
    header.push_str("\r\n");
    stream
        .write_all(header.as_bytes())
        .await
        .expect("write header");
    if !body.is_empty() {
        stream.write_all(body).await.expect("write body");
    }
    let mut buf = Vec::with_capacity(4096);
    stream.read_to_end(&mut buf).await.expect("read");

    parse_http_response_full(&buf)
}

fn parse_http_response_full(raw: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let header_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response missing CRLFCRLF terminator");
    let header_text = std::str::from_utf8(&raw[..header_end]).expect("ascii headers");
    let mut lines = header_text.lines();
    let status_line = lines.next().expect("status line");
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
    }

    let body = raw[header_end + 4..].to_vec();
    let chunked = headers.iter().any(|(k, v)| {
        k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked { decode_chunked(&body) } else { body };
    (status, headers, body)
}

/// Extract the JSON payload of the first non-empty `data:` line from an
/// SSE-encoded HTTP body.
///
/// rmcp's streamable-HTTP transport prepends a "priming" event with an empty
/// `data:` field before the real JSON-RPC response. This helper skips empty
/// data lines and returns the first one with a payload.
pub fn extract_sse_data(body: &[u8]) -> String {
    let text = std::str::from_utf8(body).expect("sse body must be utf-8");
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    panic!("no non-empty `data:` line in SSE body: {text:?}");
}

fn decode_chunked(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        // Find next CRLF (chunk size line).
        let line_end = match raw[i..].windows(2).position(|w| w == b"\r\n") {
            Some(p) => i + p,
            None => break,
        };
        let size_str = std::str::from_utf8(&raw[i..line_end]).unwrap_or("0");
        let size = usize::from_str_radix(size_str.trim(), 16).unwrap_or(0);
        i = line_end + 2;
        if size == 0 {
            break;
        }
        out.extend_from_slice(&raw[i..i + size]);
        i += size + 2; // chunk + trailing CRLF
    }
    out
}
