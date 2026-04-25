//! Shared test utilities for the config module.
//!
//! Provides env-var scoping and temp-directory helpers used by both
//! `loader.rs` and `mod.rs` tests.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

/// Execute `f` with `key` set to `val`, then restore the previous value.
///
/// # Safety
///
/// `set_var`/`remove_var` are unsafe in Rust 2024 because concurrent readers
/// may observe a torn write. Config tests must run single-threaded
/// (`cargo test -- --test-threads=1`).
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

/// Create `~/.trumpet/` inside `home` and return the path.
pub fn make_trumpet_dir(home: &TempDir) -> PathBuf {
    let dir = home.path().join(".trumpet");
    fs::create_dir_all(&dir).unwrap();
    dir
}
