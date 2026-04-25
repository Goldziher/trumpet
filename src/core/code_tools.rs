//! Tree-sitter powered code intelligence tools.
//!
//! Provides built-in skills for repository scanning, file reading, and code
//! structure parsing. All CPU-bound work is offloaded to
//! [`tokio::task::spawn_blocking`] so the async runtime stays unblocked.

use std::path::Path;

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
/// control file-size and pagination limits.
pub struct CodeTools {
    config: CodeToolsConfig,
}

impl CodeTools {
    /// Create a new [`CodeTools`] instance with the given configuration.
    pub fn new(config: CodeToolsConfig) -> Self {
        Self { config }
    }

    /// Walk the directory tree at `path` and return metadata for each source
    /// file found.
    ///
    /// Files larger than [`CodeToolsConfig::max_file_size_bytes`] or detected
    /// as binary are skipped. Results are capped at
    /// [`CodeToolsConfig::max_files_per_page`]; check [`ScanResult::truncated`]
    /// to detect overflow.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InternalUnexpected`] if the blocking task panics or I/O
    /// fails.
    pub async fn scan_repo(&self, path: &str) -> Result<ScanResult, Error> {
        let path = path.to_owned();
        let max_size = self.config.max_file_size_bytes;
        let max_files = self.config.max_files_per_page as usize;

        tokio::task::spawn_blocking(move || scan_repo_blocking(&path, max_size, max_files))
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("scan_repo task panicked: {e}"),
            })?
    }

    /// Read `path` and return its content along with detected language metadata.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InternalUnexpected`] if the file cannot be read or the
    /// blocking task panics.
    pub async fn read_file(&self, path: &str) -> Result<ReadResult, Error> {
        let path = path.to_owned();

        tokio::task::spawn_blocking(move || read_file_blocking(&path))
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("read_file task panicked: {e}"),
            })?
    }

    /// Parse `path` with tree-sitter and return the extracted code structure.
    ///
    /// Files larger than [`CodeToolsConfig::max_file_size_bytes`] are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InternalUnexpected`] if the file cannot be read,
    /// exceeds the size limit, or tree-sitter parsing fails.
    pub async fn parse_file(&self, path: &str) -> Result<ParseResult, Error> {
        let path = path.to_owned();
        let max_size = self.config.max_file_size_bytes;

        tokio::task::spawn_blocking(move || parse_file_blocking(&path, max_size))
            .await
            .map_err(|e| Error::InternalUnexpected {
                reason: format!("parse_file task panicked: {e}"),
            })?
    }
}

// ── Blocking helpers (run inside spawn_blocking) ──────────────────────────────

fn scan_repo_blocking(root: &str, max_size: u64, max_files: usize) -> Result<ScanResult, Error> {
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut total_found: usize = 0;

    visit_dir(
        Path::new(root),
        max_size,
        max_files,
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
        let metadata = entry.metadata().map_err(|e| Error::InternalUnexpected {
            reason: format!("cannot stat '{}': {e}", entry_path.display()),
        })?;

        if metadata.is_dir() {
            visit_dir(&entry_path, max_size, max_files, out, total)?;
            continue;
        }

        if !metadata.is_file() {
            continue;
        }

        let size_bytes = metadata.len();
        if size_bytes > max_size {
            tracing::debug!(path = %entry_path.display(), size_bytes, "skipping oversized file");
            continue;
        }

        // Binary detection: read probe bytes.
        let content_bytes = match std::fs::read(&entry_path) {
            Ok(b) => b,
            Err(e) => {
                tracing::debug!(path = %entry_path.display(), error = %e, "skipping unreadable file");
                continue;
            }
        };

        if is_binary(&content_bytes) {
            tracing::debug!(path = %entry_path.display(), "skipping binary file");
            continue;
        }

        *total += 1;

        let path_str = entry_path.to_string_lossy().into_owned();
        let language = detect_language_from_path(&path_str).map(title_case);
        let content_str = String::from_utf8_lossy(&content_bytes);
        let line_count = count_lines(&content_str);

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

fn read_file_blocking(path: &str) -> Result<ReadResult, Error> {
    let bytes = std::fs::read(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot read file '{path}': {e}"),
    })?;

    let content = String::from_utf8_lossy(&bytes).into_owned();
    let language = detect_language_from_path(path).map(title_case);
    let line_count = count_lines(&content);

    Ok(ReadResult {
        path: path.to_owned(),
        language,
        content,
        line_count,
    })
}

fn parse_file_blocking(path: &str, max_size: u64) -> Result<ParseResult, Error> {
    let metadata = std::fs::metadata(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot stat file '{path}': {e}"),
    })?;

    if metadata.len() > max_size {
        return Err(Error::InternalUnexpected {
            reason: format!(
                "file '{path}' exceeds max_file_size_bytes ({} > {max_size})",
                metadata.len()
            ),
        });
    }

    let bytes = std::fs::read(path).map_err(|e| Error::InternalUnexpected {
        reason: format!("cannot read file '{path}': {e}"),
    })?;

    let content = String::from_utf8_lossy(&bytes).into_owned();
    let language_name = detect_language_from_path(path);

    let items = match language_name {
        None => {
            tracing::debug!(path, "no language detected; skipping parse");
            serde_json::Value::Null
        }
        Some(lang) => {
            let config = ProcessConfig::new(lang);
            match process(&content, &config) {
                Ok(result) => {
                    serde_json::to_value(&result).map_err(|e| Error::InternalUnexpected {
                        reason: format!("failed to serialize parse result for '{path}': {e}"),
                    })?
                }
                Err(e) => {
                    tracing::warn!(path, language = lang, error = %e, "tree-sitter parse failed");
                    serde_json::Value::Null
                }
            }
        }
    };

    let language = language_name.map(title_case);
    Ok(ParseResult {
        path: path.to_owned(),
        language,
        items,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CodeToolsConfig;

    fn default_tools() -> CodeTools {
        CodeTools::new(CodeToolsConfig::default())
    }

    // Path to the trumpet project source, resolved at test time.
    fn src_path() -> String {
        let manifest = env!("CARGO_MANIFEST_DIR");
        format!("{manifest}/src")
    }

    fn lib_rs_path() -> String {
        let manifest = env!("CARGO_MANIFEST_DIR");
        format!("{manifest}/src/lib.rs")
    }

    fn types_rs_path() -> String {
        let manifest = env!("CARGO_MANIFEST_DIR");
        format!("{manifest}/src/core/types.rs")
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
        let config = CodeToolsConfig {
            max_files_per_page: 2,
            ..CodeToolsConfig::default()
        };
        let tools = CodeTools::new(config);

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
}
