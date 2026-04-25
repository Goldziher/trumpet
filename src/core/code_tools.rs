//! Tree-sitter powered code intelligence tools.
//!
//! Provides built-in tools for repository scanning, file reading, and code
//! structure parsing. All CPU-bound work is offloaded to
//! [`tokio::task::spawn_blocking`] so the async runtime stays unblocked.
//!
//! All filesystem access is sandboxed to a configurable workspace root (see
//! [`CodeToolsConfig::workspace_root`]). Caller-supplied paths are resolved
//! relative to the root, canonicalised, and rejected unless they remain
//! descendants of the root after symlink resolution.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tree_sitter_language_pack::{ProcessConfig, detect_language_from_path, process};

use crate::config::CodeToolsConfig;
use crate::error::Error;

// ── Result types ─────────────────────────────────────────────────────────────

/// Metadata for a single file discovered during a repository scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path to the file, relative to the scan root.
    pub path: String,
    /// Detected language name (title-cased), or `None` for unknown extensions.
    pub language: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Number of newline-delimited lines in the file.
    pub line_count: usize,
}

/// Result of a repository directory scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanResult {
    /// Files found, up to `max_files_per_page`.
    pub files: Vec<FileEntry>,
    /// Total number of eligible files discovered (may exceed `files.len()`).
    pub total_found: usize,
    /// `true` when `total_found` exceeds the page limit.
    pub truncated: bool,
}

/// Result of reading a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResult {
    /// Absolute or relative path used to open the file.
    pub path: String,
    /// Detected language name (title-cased), or `None` for unknown extensions.
    pub language: Option<String>,
    /// Raw UTF-8 content of the file.
    pub content: String,
    /// Number of newline-delimited lines.
    pub line_count: usize,
}

/// Result of parsing a single source file with tree-sitter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    /// Absolute or relative path used to open the file.
    pub path: String,
    /// Detected language name (title-cased), or `None` for unknown extensions.
    pub language: Option<String>,
    /// Serialized [`tree_sitter_language_pack::ProcessResult`] containing
    /// structure items, imports, exports, and metrics.
    pub items: serde_json::Value,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Title-case a language name returned by the tree-sitter language pack.
///
/// E.g. `"rust"` → `"Rust"`, `"javascript"` → `"Javascript"`.
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Return `true` if the first 512 bytes of `data` contain a null byte,
/// which is a strong signal that the file is binary.
fn is_binary(data: &[u8]) -> bool {
    let probe = if data.len() > 512 { &data[..512] } else { data };
    probe.contains(&0u8)
}

/// Count newline characters as a fast line counter that avoids a full
/// [`str::lines`] allocation.
fn count_lines(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }
    let newlines = content.as_bytes().iter().filter(|&&b| b == b'\n').count();
    // A file with no trailing newline still has at least one line.
    if content.ends_with('\n') {
        newlines
    } else {
        newlines + 1
    }
}

// ── CodeTools ─────────────────────────────────────────────────────────────────

/// Built-in code intelligence tools backed by tree-sitter.
///
/// Construct with [`CodeTools::new`] and supply a [`CodeToolsConfig`] to
/// control file-size, pagination limits, and the workspace sandbox root.
#[derive(Debug)]
pub struct CodeTools {
    config: CodeToolsConfig,
    /// Canonicalised sandbox root. All caller-supplied paths must resolve
    /// to a descendant of this directory.
    workspace_root: PathBuf,
}

impl CodeTools {
    /// Create a new [`CodeTools`] instance with the given configuration.
    ///
    /// The configured `workspace_root` is canonicalised at construction time
    /// to ensure later sandbox checks compare against an absolute path with
    /// symlinks resolved.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigValidationFailed`] when `workspace_root` is
    /// `None` or fails to canonicalise (e.g. the directory does not exist).
    pub fn new(config: CodeToolsConfig) -> Result<Self, Error> {
        let raw_root =
            config
                .workspace_root
                .as_ref()
                .ok_or_else(|| Error::ConfigValidationFailed {
                    reason: "code_tools.workspace_root is required".to_owned(),
                })?;
        let workspace_root =
            std::fs::canonicalize(raw_root).map_err(|e| Error::ConfigValidationFailed {
                reason: format!(
                    "code_tools.workspace_root '{}' is not a valid directory: {e}",
                    raw_root.display()
                ),
            })?;
        Ok(Self {
            config,
            workspace_root,
        })
    }

    /// Resolve a caller-supplied path against the workspace sandbox.
    ///
    /// Relative paths are joined with the workspace root; absolute paths are
    /// taken as-is. The result is then canonicalised (resolving symlinks)
    /// and checked to be a descendant of the workspace root.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ToolInvocationFailed`] when the path cannot be
    /// canonicalised (typically a missing file) or escapes the sandbox root.
    fn resolve_safe_path(&self, tool_name: &str, raw: &str) -> Result<PathBuf, Error> {
        let candidate = {
            let p = Path::new(raw);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                self.workspace_root.join(p)
            }
        };
        let canonical =
            std::fs::canonicalize(&candidate).map_err(|e| Error::ToolInvocationFailed {
                name: tool_name.to_owned(),
                reason: format!("path '{raw}' could not be resolved: {e}"),
            })?;
        if !canonical.starts_with(&self.workspace_root) {
            return Err(Error::ToolInvocationFailed {
                name: tool_name.to_owned(),
                reason: format!(
                    "path '{raw}' resolves outside the workspace sandbox '{}'",
                    self.workspace_root.display()
                ),
            });
        }
        Ok(canonical)
    }

    /// Walk the directory tree at `path` and return metadata for each source
    /// file found.
    ///
    /// `path` is resolved against the configured workspace sandbox; paths
    /// that escape the sandbox are rejected.
    ///
    /// Files larger than [`CodeToolsConfig::max_file_size_bytes`] or detected
    /// as binary are skipped. Results are capped at
    /// [`CodeToolsConfig::max_files_per_page`]; check [`ScanResult::truncated`]
    /// to detect overflow.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ToolInvocationFailed`] when sandbox resolution fails,
    /// or [`Error::InternalUnexpected`] if the blocking task panics or I/O
    /// fails.
    pub async fn scan_repo(&self, path: &str) -> Result<ScanResult, Error> {
        let resolved = self.resolve_safe_path("code.scan_repo", path)?;
        let max_size = self.config.max_file_size_bytes;
        let max_files = self.config.max_files_per_page as usize;
        let skip_dirs = self.config.walk_skip_dirs.clone();

        tokio::task::spawn_blocking(move || {
            scan_repo_blocking(&resolved, max_size, max_files, &skip_dirs)
        })
        .await
        .map_err(|e| Error::InternalUnexpected {
            reason: format!("scan_repo task panicked: {e}"),
        })?
    }

    /// Read `path` and return its content along with detected language metadata.
    ///
    /// `path` is resolved against the configured workspace sandbox; paths
    /// that escape the sandbox are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ToolInvocationFailed`] when sandbox resolution fails,
    /// or [`Error::InternalUnexpected`] if the file cannot be read or the
    /// blocking task panics.
    pub async fn read_file(&self, path: &str) -> Result<ReadResult, Error> {
        let resolved = self.resolve_safe_path("code.read_file", path)?;
        let max_size = self.config.max_file_size_bytes;

        tokio::task::spawn_blocking(move || read_file_blocking(&resolved, max_size))
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("read_file task panicked: {e}"),
            })?
    }

    /// Parse `path` with tree-sitter and return the extracted code structure.
    ///
    /// `path` is resolved against the configured workspace sandbox; paths
    /// that escape the sandbox are rejected. Files larger than
    /// [`CodeToolsConfig::max_file_size_bytes`] are also rejected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ToolInvocationFailed`] when sandbox resolution fails,
    /// or [`Error::InternalUnexpected`] if the file cannot be read,
    /// exceeds the size limit, or tree-sitter parsing fails.
    pub async fn parse_file(&self, path: &str) -> Result<ParseResult, Error> {
        let resolved = self.resolve_safe_path("code.parse_file", path)?;
        let max_size = self.config.max_file_size_bytes;

        tokio::task::spawn_blocking(move || parse_file_blocking(&resolved, max_size))
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("parse_file task panicked: {e}"),
            })?
    }

    /// Dispatch a tool invocation by name.
    ///
    /// Extracts `path` from the input JSON and routes to the appropriate
    /// method. Returns the result serialized as JSON.
    pub async fn dispatch(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, Error> {
        let path = input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            Error::ToolInvocationFailed {
                name: name.to_owned(),
                reason: "missing required 'path' parameter".to_owned(),
            }
        })?;

        match name {
            "code.scan_repo" => {
                let result = self.scan_repo(path).await?;
                serde_json::to_value(result).map_err(|e| Error::ToolInvocationFailed {
                    name: name.to_owned(),
                    reason: format!("serialization failed: {e}"),
                })
            }
            "code.read_file" => {
                let result = self.read_file(path).await?;
                serde_json::to_value(result).map_err(|e| Error::ToolInvocationFailed {
                    name: name.to_owned(),
                    reason: format!("serialization failed: {e}"),
                })
            }
            "code.parse_file" => {
                let result = self.parse_file(path).await?;
                serde_json::to_value(result).map_err(|e| Error::ToolInvocationFailed {
                    name: name.to_owned(),
                    reason: format!("serialization failed: {e}"),
                })
            }
            _ => Err(Error::ToolNotFound {
                name: name.to_owned(),
            }),
        }
    }
}

// ── Blocking helpers (run inside spawn_blocking) ──────────────────────────────

fn scan_repo_blocking(
    root: &Path,
    max_size: u64,
    max_files: usize,
    skip_dirs: &[String],
) -> Result<ScanResult, Error> {
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut total_found: usize = 0;

    visit_dir(
        root,
        max_size,
        max_files,
        skip_dirs,
        &mut entries,
        &mut total_found,
    )?;

    let truncated = total_found > max_files;
    Ok(ScanResult {
        files: entries,
        total_found,
        truncated,
    })
}

/// Recursive directory visitor that appends to `out`, respecting `max_files`.
fn visit_dir(
    dir: &Path,
    max_size: u64,
    max_files: usize,
    skip_dirs: &[String],
    out: &mut Vec<FileEntry>,
    total: &mut usize,
) -> Result<(), Error> {
    let read_dir = std::fs::read_dir(dir).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot read directory '{}': {e}", dir.display()),
    })?;

    for result in read_dir {
        let entry = result.map_err(|e| Error::InternalUnexpected {
            reason: format!("directory entry error in '{}': {e}", dir.display()),
        })?;
        let entry_path = entry.path();

        // Use symlink_metadata to avoid following symlinks (prevents loops).
        let sym_meta =
            std::fs::symlink_metadata(&entry_path).map_err(|e| Error::InternalUnexpected {
                reason: format!("cannot stat '{}': {e}", entry_path.display()),
            })?;

        if sym_meta.file_type().is_symlink() {
            continue;
        }

        if sym_meta.is_dir() {
            // Skip hidden directories (.git, .vscode, …) and configured deny
            // list (target, node_modules, …). Without this, a default scan
            // of any non-trivial repo enumerates millions of dependency
            // files and is effectively unusable.
            let dir_name = entry_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if dir_name.starts_with('.') {
                tracing::trace!(path = %entry_path.display(), "skipping hidden directory");
                continue;
            }
            if skip_dirs.iter().any(|d| d == dir_name) {
                tracing::trace!(path = %entry_path.display(), "skipping deny-listed directory");
                continue;
            }
            visit_dir(&entry_path, max_size, max_files, skip_dirs, out, total)?;
            continue;
        }

        if !sym_meta.is_file() {
            continue;
        }

        let size_bytes = sym_meta.len();
        if size_bytes > max_size {
            tracing::debug!(path = %entry_path.display(), size_bytes, "skipping oversized file");
            continue;
        }

        // Read just the binary-detection probe (first 512 bytes) without
        // loading the whole file into memory.
        let mut file = match std::fs::File::open(&entry_path) {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!(path = %entry_path.display(), error = %e, "skipping unreadable file");
                continue;
            }
        };
        let mut probe = [0u8; 512];
        let probe_len = match std::io::Read::read(&mut file, &mut probe) {
            Ok(n) => n,
            Err(e) => {
                tracing::debug!(path = %entry_path.display(), error = %e, "skipping unreadable file");
                continue;
            }
        };
        if is_binary(&probe[..probe_len]) {
            tracing::debug!(path = %entry_path.display(), "skipping binary file");
            continue;
        }

        // Stream-count newlines through a buffered reader rather than
        // re-reading the entire file into memory.
        let line_count = match count_lines_streaming(&entry_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(path = %entry_path.display(), error = %e, "skipping unreadable file");
                continue;
            }
        };

        *total += 1;

        let path_str = entry_path.to_string_lossy().into_owned();
        let language = detect_language_from_path(&path_str).map(title_case);

        if out.len() < max_files {
            out.push(FileEntry {
                path: path_str,
                language,
                size_bytes,
                line_count,
            });
        }
    }

    Ok(())
}

/// Count newline-delimited lines in `path` using a buffered reader so the
/// full content never needs to live in memory.
///
/// Matches the trailing-newline semantics of [`count_lines`]: a non-empty
/// file with no trailing `\n` still counts as having a final line.
fn count_lines_streaming(path: &Path) -> std::io::Result<usize> {
    use std::io::{BufReader, Read as _};

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(8 * 1024, file);
    let mut buf = [0u8; 8 * 1024];
    let mut newlines: usize = 0;
    let mut last_byte: Option<u8> = None;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b == b'\n' {
                newlines += 1;
            }
        }
        last_byte = Some(buf[n - 1]);
    }
    let lines = match last_byte {
        None => 0,               // empty file
        Some(b'\n') => newlines, // trailing newline
        Some(_) => newlines + 1, // final unterminated line
    };
    Ok(lines)
}

fn read_file_blocking(path: &Path, max_size: u64) -> Result<ReadResult, Error> {
    let path_str = path.display().to_string();
    let meta = std::fs::metadata(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot stat file '{path_str}': {e}"),
    })?;
    if meta.len() > max_size {
        return Err(Error::InternalUnexpected {
            reason: format!(
                "file '{path_str}' exceeds max_file_size_bytes ({} > {max_size})",
                meta.len()
            ),
        });
    }
    let bytes = std::fs::read(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot read file '{path_str}': {e}"),
    })?;

    let content = String::from_utf8_lossy(&bytes).into_owned();
    let language = detect_language_from_path(&path_str).map(title_case);
    let line_count = count_lines(&content);

    Ok(ReadResult {
        path: path_str,
        language,
        content,
        line_count,
    })
}

fn parse_file_blocking(path: &Path, max_size: u64) -> Result<ParseResult, Error> {
    let path_str = path.display().to_string();
    let metadata = std::fs::metadata(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot stat file '{path_str}': {e}"),
    })?;

    if metadata.len() > max_size {
        return Err(Error::InternalUnexpected {
            reason: format!(
                "file '{path_str}' exceeds max_file_size_bytes ({} > {max_size})",
                metadata.len()
            ),
        });
    }

    let bytes = std::fs::read(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot read file '{path_str}': {e}"),
    })?;

    let content = String::from_utf8_lossy(&bytes).into_owned();
    let language_name = detect_language_from_path(&path_str);

    let items = match language_name {
        None => {
            tracing::debug!(path = %path_str, "no language detected; skipping parse");
            serde_json::Value::Null
        }
        Some(lang) => {
            let config = ProcessConfig::new(lang);
            match process(&content, &config) {
                Ok(result) => {
                    serde_json::to_value(&result).map_err(|e| Error::InternalUnexpected {
                        reason: format!("failed to serialize parse result for '{path_str}': {e}"),
                    })?
                }
                Err(e) => {
                    tracing::warn!(path = %path_str, language = lang, error = %e, "tree-sitter parse failed");
                    serde_json::Value::Null
                }
            }
        }
    };

    let language = language_name.map(title_case);
    Ok(ParseResult {
        path: path_str,
        language,
        items,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::CodeToolsConfig;

    /// Construct `CodeTools` rooted at `CARGO_MANIFEST_DIR` so tests can
    /// reference real source files in the trumpet repo via relative paths.
    fn default_tools() -> CodeTools {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        CodeTools::new(CodeToolsConfig {
            workspace_root: Some(manifest),
            ..CodeToolsConfig::default()
        })
        .expect("CodeTools::new must succeed for the manifest dir")
    }

    /// Path to the trumpet project source directory, relative to the
    /// workspace root.
    fn src_path() -> String {
        "src".to_owned()
    }

    fn lib_rs_path() -> String {
        "src/lib.rs".to_owned()
    }

    fn types_rs_path() -> String {
        "src/core/types.rs".to_owned()
    }

    // ── scan_repo ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn scan_repo_finds_rust_files() {
        let tools = default_tools();
        let result = tools
            .scan_repo(&src_path())
            .await
            .expect("scan_repo must succeed");

        assert!(
            !result.files.is_empty(),
            "scan must return at least one file in src/"
        );

        let rust_files: Vec<_> = result
            .files
            .iter()
            .filter(|f| f.path.ends_with(".rs"))
            .collect();

        assert!(!rust_files.is_empty(), "must find .rs files in src/");

        for file in &rust_files {
            assert_eq!(
                file.language.as_deref(),
                Some("Rust"),
                "language for .rs file must be 'Rust', got {:?} for {}",
                file.language,
                file.path
            );
        }
    }

    #[tokio::test]
    async fn scan_repo_respects_max_files() {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let config = CodeToolsConfig {
            max_files_per_page: 2,
            workspace_root: Some(manifest),
            ..CodeToolsConfig::default()
        };
        let tools = CodeTools::new(config).expect("CodeTools::new");

        let result = tools
            .scan_repo(&src_path())
            .await
            .expect("scan_repo must succeed");

        assert!(
            result.files.len() <= 2,
            "files.len() must not exceed max_files_per_page=2, got {}",
            result.files.len()
        );
        assert!(
            result.truncated,
            "result must be marked truncated when src/ has more than 2 files"
        );
        assert!(
            result.total_found > 2,
            "total_found must reflect the actual count, got {}",
            result.total_found
        );
    }

    // ── read_file ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn read_file_returns_content() {
        let tools = default_tools();
        let result = tools
            .read_file(&lib_rs_path())
            .await
            .expect("read_file must succeed");

        assert!(
            !result.content.is_empty(),
            "content of src/lib.rs must be non-empty"
        );
        assert_eq!(
            result.language.as_deref(),
            Some("Rust"),
            "language for src/lib.rs must be 'Rust'"
        );
        assert!(result.line_count > 0, "line_count must be positive");
    }

    // ── parse_file ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn parse_file_returns_items() {
        let tools = default_tools();
        let result = tools
            .parse_file(&types_rs_path())
            .await
            .expect("parse_file must succeed");

        assert_eq!(
            result.language.as_deref(),
            Some("Rust"),
            "language for types.rs must be 'Rust'"
        );
        assert!(
            !result.items.is_null(),
            "parse items for types.rs must not be null"
        );

        // The structure field should contain at least one item (types.rs has many structs/impls).
        let structure = result.items.get("structure");
        assert!(
            structure.is_some(),
            "parse result must contain a 'structure' key"
        );
        let arr = structure.unwrap().as_array();
        assert!(
            arr.is_some_and(|a| !a.is_empty()),
            "structure array must be non-empty for types.rs"
        );
    }

    // ── dispatch ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn dispatch_code_read_file_succeeds() {
        let tools = default_tools();
        let input = serde_json::json!({"path": lib_rs_path()});
        let result = tools
            .dispatch("code.read_file", input)
            .await
            .expect("dispatch must succeed for code.read_file");

        assert!(
            result.get("content").is_some(),
            "result must have content field"
        );
        assert!(
            result.get("language").is_some(),
            "result must have language field"
        );
    }

    #[tokio::test]
    async fn dispatch_unknown_tool_returns_error() {
        let tools = default_tools();
        let input = serde_json::json!({"path": "/tmp"});
        let result = tools.dispatch("unknown.tool", input).await;
        assert!(result.is_err(), "unknown tool name must return error");
    }

    #[tokio::test]
    async fn dispatch_missing_path_returns_error() {
        let tools = default_tools();
        let input = serde_json::json!({});
        let result = tools.dispatch("code.read_file", input).await;
        assert!(result.is_err(), "missing path must return error");
    }

    // ── sandbox ────────────────────────────────────────────────────────────────

    #[test]
    fn new_rejects_missing_workspace_root() {
        let err = CodeTools::new(CodeToolsConfig::default())
            .expect_err("CodeTools::new must fail when workspace_root is None");
        assert!(
            matches!(err, Error::ConfigValidationFailed { .. }),
            "expected ConfigValidationFailed, got: {err:?}"
        );
    }

    #[test]
    fn new_rejects_nonexistent_workspace_root() {
        let err = CodeTools::new(CodeToolsConfig {
            workspace_root: Some(PathBuf::from("/this/does/not/exist/anywhere")),
            ..CodeToolsConfig::default()
        })
        .expect_err("CodeTools::new must fail for a missing directory");
        assert!(
            matches!(err, Error::ConfigValidationFailed { .. }),
            "expected ConfigValidationFailed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn read_file_rejects_traversal_attack() {
        let tools = default_tools();
        // `..` to escape the workspace root by climbing above the manifest dir.
        let result = tools.read_file("../../etc/passwd").await;
        let err = result.expect_err("traversal attack must be rejected");
        match err {
            Error::ToolInvocationFailed { reason, .. } => {
                assert!(
                    reason.contains("outside the workspace sandbox")
                        || reason.contains("could not be resolved"),
                    "expected sandbox or resolution error, got: {reason}"
                );
            }
            other => panic!("expected ToolInvocationFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_file_rejects_absolute_path_outside_root() {
        let tools = default_tools();
        let result = tools.read_file("/etc/hostname").await;
        // /etc/hostname may exist on macOS (rare) but is outside CARGO_MANIFEST_DIR;
        // either way the call must fail (sandbox or unresolved).
        let err = result.expect_err("absolute path outside root must be rejected");
        assert!(
            matches!(err, Error::ToolInvocationFailed { .. }),
            "expected ToolInvocationFailed, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn read_file_rejects_symlink_escape() {
        use tempfile::TempDir;

        let workspace = TempDir::new().expect("tempdir");
        let outside = TempDir::new().expect("tempdir");
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "classified").expect("write secret");

        // Place a symlink inside the workspace pointing to the file outside.
        let link = workspace.path().join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).expect("create symlink");
        #[cfg(not(unix))]
        return;

        let tools = CodeTools::new(CodeToolsConfig {
            workspace_root: Some(workspace.path().to_path_buf()),
            ..CodeToolsConfig::default()
        })
        .expect("CodeTools::new");

        let err = tools
            .read_file("escape")
            .await
            .expect_err("symlink escape must be rejected");
        match err {
            Error::ToolInvocationFailed { reason, .. } => {
                assert!(
                    reason.contains("outside the workspace sandbox"),
                    "expected sandbox error after symlink resolution, got: {reason}"
                );
            }
            other => panic!("expected ToolInvocationFailed, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_file_accepts_valid_relative_path() {
        let tools = default_tools();
        let result = tools
            .read_file("Cargo.toml")
            .await
            .expect("relative path inside workspace must succeed");
        assert!(
            !result.content.is_empty(),
            "Cargo.toml content must be non-empty"
        );
    }

    #[tokio::test]
    async fn scan_repo_rejects_traversal() {
        let tools = default_tools();
        let err = tools
            .scan_repo("../..")
            .await
            .expect_err("traversal in scan_repo must be rejected");
        assert!(
            matches!(err, Error::ToolInvocationFailed { .. }),
            "expected ToolInvocationFailed, got: {err:?}"
        );
    }

    #[test]
    fn count_lines_streaming_matches_count_lines_for_typical_file() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        let cases = [
            ("", 0usize),
            ("hello", 1),
            ("hello\n", 1),
            ("a\nb\nc\n", 3),
            ("a\nb\nc", 3),
        ];
        for (content, expected) in cases {
            let path = dir.path().join("f.txt");
            std::fs::write(&path, content).unwrap();
            let stream_count = count_lines_streaming(&path).unwrap();
            assert_eq!(
                stream_count, expected,
                "streaming count must match expected for {content:?}"
            );
            assert_eq!(
                stream_count,
                count_lines(content),
                "streaming and in-memory counts must agree for {content:?}"
            );
        }
    }

    #[tokio::test]
    async fn scan_repo_skips_hidden_and_deny_listed_dirs() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        // Visible source file at root.
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        // Hidden directory with a file that should be skipped.
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: foo").unwrap();
        // Deny-listed directory.
        std::fs::create_dir(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("target/x.rs"), "fn x() {}").unwrap();
        // Plain dir that must be visited.
        std::fs::create_dir(dir.path().join("nested")).unwrap();
        std::fs::write(dir.path().join("nested/lib.rs"), "fn lib() {}").unwrap();

        let tools = CodeTools::new(CodeToolsConfig {
            workspace_root: Some(dir.path().to_path_buf()),
            ..CodeToolsConfig::default()
        })
        .expect("CodeTools::new");

        let result = tools.scan_repo(".").await.expect("scan must succeed");
        let paths: Vec<&str> = result.files.iter().map(|f| f.path.as_str()).collect();
        assert!(
            paths.iter().any(|p| p.ends_with("main.rs")),
            "must include main.rs, got: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("nested/lib.rs")),
            "must include nested/lib.rs, got: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("/.git/")),
            "must skip .git/, got: {paths:?}"
        );
        assert!(
            !paths.iter().any(|p| p.contains("/target/")),
            "must skip target/, got: {paths:?}"
        );
    }

    #[tokio::test]
    async fn parse_file_rejects_traversal() {
        let tools = default_tools();
        let err = tools
            .parse_file("../../etc/passwd")
            .await
            .expect_err("traversal in parse_file must be rejected");
        assert!(
            matches!(err, Error::ToolInvocationFailed { .. }),
            "expected ToolInvocationFailed, got: {err:?}"
        );
    }
}
