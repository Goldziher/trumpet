//! Local-process authentication for the Trumpet daemon.
//!
//! Two complementary checks gate access to the daemon:
//!
//! 1. [`PeerCheckedUnixListener`] wraps the REST/WebSocket Unix-domain
//!    listener and rejects any incoming connection whose peer UID does not
//!    match the daemon owner. This is enforced at accept time, before any
//!    HTTP framing is read.
//! 2. [`bearer_token_interceptor`] is a tonic [`Interceptor`] that requires
//!    every gRPC request to present `Authorization: Bearer <token>`, where
//!    `<token>` matches the contents of the daemon's auth-token file.
//!
//! The token file is auto-generated on startup (see [`load_or_create_token`])
//! with `0600` permissions; only the daemon owner can read it.

use std::path::Path;
use std::sync::Arc;

use axum::serve::Listener;
use tokio::net::{UnixListener, UnixStream, unix::SocketAddr};
use tonic::{Status, service::Interceptor};

use crate::error::Error;

// ── Token storage ─────────────────────────────────────────────────────────────

/// Read the daemon auth token from disk, generating it if missing.
///
/// On creation, the file is written with `0600` permissions so only the
/// daemon owner can read it. The token itself is a UUID v4 (122 bits of
/// entropy), formatted as 36 hex characters with hyphens.
///
/// # Errors
///
/// Returns [`Error::InternalUnexpected`] when the parent directory cannot
/// be created, the file cannot be read, or the file cannot be written with
/// the required permissions.
pub fn load_or_create_token(path: &Path) -> Result<String, Error> {
    if path.exists() {
        let token = std::fs::read_to_string(path).map_err(|e| Error::InternalUnexpected {
            reason: format!("cannot read auth token at '{}': {e}", path.display()),
        })?;
        let trimmed = token.trim().to_owned();
        if trimmed.is_empty() {
            return Err(Error::InternalUnexpected {
                reason: format!("auth token at '{}' is empty", path.display()),
            });
        }
        return Ok(trimmed);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::InternalUnexpected {
            reason: format!(
                "cannot create auth token directory '{}': {e}",
                parent.display()
            ),
        })?;
    }

    let token = uuid::Uuid::new_v4().to_string();
    write_token_with_owner_only_perms(path, &token)?;
    Ok(token)
}

#[cfg(unix)]
fn write_token_with_owner_only_perms(path: &Path, token: &str) -> Result<(), Error> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("cannot create auth token file '{}': {e}", path.display()),
        })?;
    file.write_all(token.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("cannot write auth token to '{}': {e}", path.display()),
        })?;
    Ok(())
}

#[cfg(not(unix))]
fn write_token_with_owner_only_perms(path: &Path, token: &str) -> Result<(), Error> {
    std::fs::write(path, token).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot write auth token to '{}': {e}", path.display()),
    })
}

// ── Unix peer-credential listener ─────────────────────────────────────────────

/// Process UID for the current daemon, used to compare against incoming
/// connections' peer credentials.
#[cfg(unix)]
pub fn current_uid() -> u32 {
    // SAFETY: `getuid` has no preconditions and never fails.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
pub fn current_uid() -> u32 {
    0
}

/// Wraps a [`UnixListener`] and rejects connections whose peer UID does not
/// match `expected_uid`. Implements [`axum::serve::Listener`] so it slots
/// directly into [`axum::serve`].
pub struct PeerCheckedUnixListener {
    inner: UnixListener,
    expected_uid: u32,
    require_auth: bool,
}

impl PeerCheckedUnixListener {
    /// Build a peer-checking wrapper around an existing [`UnixListener`].
    ///
    /// When `require_auth` is `false`, every accepted connection is passed
    /// through without a UID check (intended only for tests or local
    /// development where the daemon is intentionally open).
    pub fn new(inner: UnixListener, expected_uid: u32, require_auth: bool) -> Self {
        Self {
            inner,
            expected_uid,
            require_auth,
        }
    }
}

impl Listener for PeerCheckedUnixListener {
    type Io = UnixStream;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let (io, addr) = Listener::accept(&mut self.inner).await;

            if !self.require_auth {
                return (io, addr);
            }

            match io.peer_cred() {
                Ok(cred) if cred.uid() == self.expected_uid => return (io, addr),
                Ok(cred) => {
                    tracing::warn!(
                        peer_uid = cred.uid(),
                        expected_uid = self.expected_uid,
                        "rejecting unix connection from foreign uid"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "could not read peer credentials; rejecting connection"
                    );
                }
            }
            // `io` drops here, closing the socket.
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

// ── gRPC bearer-token interceptor ─────────────────────────────────────────────

/// Tonic [`Interceptor`] that enforces `Authorization: Bearer <token>` on
/// every incoming gRPC request.
///
/// The expected token is shared via [`Arc`] so the interceptor can be cloned
/// cheaply for each connection. When `require_auth` is `false`, all requests
/// are allowed through.
#[derive(Clone)]
pub struct BearerTokenInterceptor {
    expected: Arc<String>,
    require_auth: bool,
}

impl BearerTokenInterceptor {
    /// Create a new interceptor that compares the incoming `Authorization`
    /// header against `Bearer <token>`.
    pub fn new(token: Arc<String>, require_auth: bool) -> Self {
        Self {
            expected: token,
            require_auth,
        }
    }
}

impl Interceptor for BearerTokenInterceptor {
    fn call(&mut self, request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        if !self.require_auth {
            return Ok(request);
        }

        let provided = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let expected = format!("Bearer {}", *self.expected);

        // Constant-time comparison would be ideal, but the token is sent in
        // plaintext over a localhost loopback already; timing leaks here are
        // not the weakest link.
        if provided == expected {
            Ok(request)
        } else {
            Err(Status::unauthenticated(
                "missing or invalid bearer token in 'authorization' metadata",
            ))
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_or_create_token_generates_when_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.token");
        let token = load_or_create_token(&path).expect("token must be generated");
        assert_eq!(
            token.len(),
            36,
            "UUID v4 token must be 36 chars, got: {token}"
        );
        assert!(path.exists(), "token file must be created on disk");
    }

    #[test]
    fn load_or_create_token_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.token");
        let first = load_or_create_token(&path).expect("first call generates");
        let second = load_or_create_token(&path).expect("second call reads");
        assert_eq!(first, second, "token must persist across calls");
    }

    #[cfg(unix)]
    #[test]
    fn token_file_has_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.token");
        load_or_create_token(&path).expect("token must be generated");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "token file must be 0600 (owner read/write only)"
        );
    }

    #[test]
    fn load_or_create_token_rejects_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.token");
        std::fs::write(&path, "").unwrap();
        let err = load_or_create_token(&path).expect_err("empty token must be rejected");
        assert!(
            matches!(err, Error::InternalUnexpected { ref reason } if reason.contains("empty")),
            "expected InternalUnexpected mentioning 'empty', got: {err:?}"
        );
    }

    #[test]
    fn bearer_interceptor_rejects_missing_header() {
        let mut interceptor =
            BearerTokenInterceptor::new(Arc::new("secret-token".to_owned()), true);
        let request = tonic::Request::new(());
        let err = interceptor
            .call(request)
            .expect_err("missing header must be rejected");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn bearer_interceptor_rejects_wrong_token() {
        let mut interceptor =
            BearerTokenInterceptor::new(Arc::new("secret-token".to_owned()), true);
        let mut request = tonic::Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer wrong-token".parse().unwrap());
        let err = interceptor
            .call(request)
            .expect_err("wrong token must be rejected");
        assert_eq!(err.code(), tonic::Code::Unauthenticated);
    }

    #[test]
    fn bearer_interceptor_accepts_correct_token() {
        let mut interceptor =
            BearerTokenInterceptor::new(Arc::new("secret-token".to_owned()), true);
        let mut request = tonic::Request::new(());
        request
            .metadata_mut()
            .insert("authorization", "Bearer secret-token".parse().unwrap());
        let result = interceptor.call(request);
        assert!(result.is_ok(), "correct token must be accepted");
    }

    #[test]
    fn bearer_interceptor_skips_check_when_disabled() {
        let mut interceptor =
            BearerTokenInterceptor::new(Arc::new("secret-token".to_owned()), false);
        let request = tonic::Request::new(());
        // No header at all — must still pass through.
        let result = interceptor.call(request);
        assert!(
            result.is_ok(),
            "interceptor must allow all when require_auth=false"
        );
    }
}
