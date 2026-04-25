//! Shared test utilities for the config module.
//!
//! Provides env-var scoping, cwd scoping, and temp-directory helpers.
//! All tests that use these helpers must be annotated with
//! `#[serial_test::serial]` to prevent concurrent env/cwd mutations.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

/// Execute `f` with `key` set to `val`, then restore the previous value.
///
/// # Safety
///
/// `set_var`/`remove_var` are unsafe in Rust 2024 because concurrent readers
/// may observe a torn write. Callers must use `#[serial_test::serial]`.
pub fn with_env<F: FnOnce()>(key: &str, val: &str, f: F) {
    let prev = std::env::var(key).ok();
    unsafe { std::env::set_var(key, val) };
    f();
    match prev {
        Some(v) => unsafe { std::env::set_var(key, v) },
        None => unsafe { std::env::remove_var(key) },
    }
}

/// Execute `f` with `$HOME` pointed at `dir`.
pub fn with_home<F: FnOnce()>(dir: &TempDir, f: F) {
    with_env("HOME", dir.path().to_str().unwrap(), f);
}

/// Execute `f` with the working directory set to `dir`, restoring on return.
///
/// Uses a drop guard so the cwd is restored even on panic.
pub fn with_cwd<F: FnOnce()>(dir: &Path, f: F) {
    let original = std::env::current_dir().unwrap();
    struct CwdGuard(PathBuf);
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }
    let _guard = CwdGuard(original);
    std::env::set_current_dir(dir).unwrap();
    f();
}

/// Create `~/.trumpet/` inside `home` and return the path.
pub fn make_trumpet_dir(home: &TempDir) -> PathBuf {
    let dir = home.path().join(".trumpet");
    fs::create_dir_all(&dir).unwrap();
    dir
}
