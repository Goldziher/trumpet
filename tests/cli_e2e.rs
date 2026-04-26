//! End-to-end CLI tests that spawn the `trumpet` binary against a live daemon.
//!
//! Each test:
//! 1. Spawns an in-process daemon via [`TestDaemon`].
//! 2. Creates a temporary HOME directory with a `~/.trumpet/config.toml`
//!    pointing at that daemon's socket.
//! 3. Runs `trumpet` subcommands via [`assert_cmd::Command`], passing
//!    `HOME=<temp>` so `Config::load()` finds the right socket.
//! 4. Shuts down the daemon.

mod common;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use assert_cmd::Command;
use tempfile::TempDir;

use common::TestDaemon;

// ── Config-file helper ────────────────────────────────────────────────────────

/// Write `~/.trumpet/config.toml` inside `home_dir` so the binary connects to
/// `socket_path` and reads the auth token from `auth_token_path`.
fn write_client_config(home_dir: &TempDir, daemon: &TestDaemon) {
    let trumpet_dir = home_dir.path().join(".trumpet");
    fs::create_dir_all(&trumpet_dir).expect("create .trumpet dir");

    let socket_path = daemon.socket_path.display().to_string();
    let auth_token_path = daemon.config.security.auth_token_path.display().to_string();
    let pid_path = daemon.config.daemon.pid_file.display().to_string();

    let config_toml = format!(
        r#"[daemon]
socket_path = "{socket_path}"
pid_file = "{pid_path}"

[security]
auth_token_path = "{auth_token_path}"
require_auth = false
"#
    );

    fs::write(trumpet_dir.join("config.toml"), config_toml).expect("write client config.toml");
}

/// Build an `assert_cmd::Command` for the `trumpet` binary with `HOME`
/// pointed at `home_dir`.
fn trumpet_cmd(home_dir: &TempDir) -> Command {
    let mut cmd = Command::cargo_bin("trumpet").expect("trumpet binary must build");
    cmd.env("HOME", home_dir.path());
    cmd
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_register_and_list_round_trip() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;
    let home = TempDir::new().expect("create temp home");
    write_client_config(&home, &daemon);

    trumpet_cmd(&home)
        .args([
            "agent",
            "register",
            "--name",
            "foo",
            "--tags",
            "review,lint",
        ])
        .assert()
        .success();

    let output = trumpet_cmd(&home)
        .args(["agent", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let agents: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("agent list must be valid JSON");

    assert!(
        !agents.is_empty(),
        "agent list must contain at least the registered agent"
    );
    assert!(
        agents
            .iter()
            .any(|a| a.get("name").and_then(|n| n.as_str()) == Some("foo")),
        "registered agent 'foo' must appear in the list"
    );

    daemon
        .shutdown()
        .await
        .expect("daemon must shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_submit_and_get() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;
    let home = TempDir::new().expect("create temp home");
    write_client_config(&home, &daemon);

    // Register an agent so we have a valid agent to assign work to.
    trumpet_cmd(&home)
        .args(["agent", "register", "--name", "worker"])
        .assert()
        .success();

    // Submit a task and capture the ID.
    let output = trumpet_cmd(&home)
        .args(["task", "submit", "--message", "do something", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let task: serde_json::Value =
        serde_json::from_slice(&output).expect("task submit must return valid JSON");
    let task_id = task
        .get("id")
        .and_then(|v| v.as_str())
        .expect("task must have an id");

    // Retrieve it by ID.
    let output = trumpet_cmd(&home)
        .args(["task", "get", task_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let fetched: serde_json::Value =
        serde_json::from_slice(&output).expect("task get must return valid JSON");
    assert_eq!(
        fetched.get("id").and_then(|v| v.as_str()),
        Some(task_id),
        "fetched task id must match submitted task id"
    );

    daemon
        .shutdown()
        .await
        .expect("daemon must shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_list_emits_built_ins() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;
    let home = TempDir::new().expect("create temp home");
    write_client_config(&home, &daemon);

    let output = trumpet_cmd(&home)
        .args(["tool", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains("code.scan_repo"),
        "tool list must include the built-in 'code.scan_repo' tool, got: {text}"
    );

    daemon
        .shutdown()
        .await
        .expect("daemon must shut down cleanly");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_show_prints_token() {
    let daemon = TestDaemon::builder().require_auth(false).spawn().await;
    let home = TempDir::new().expect("create temp home");
    write_client_config(&home, &daemon);

    let output = trumpet_cmd(&home)
        .args(["auth", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8_lossy(&output);
    assert!(
        text.contains(&daemon.auth_token),
        "auth show must print the daemon's auth token, got: {text}"
    );

    daemon
        .shutdown()
        .await
        .expect("daemon must shut down cleanly");
}

// Serial because of the SIGHUP-raising test in `auth_rotate_e2e` — a process-
// wide signal can terminate the child subprocess this test spawns when both
// tests run in parallel. Use a named serial mutex shared with that test
// (cargo runs each integration-test binary in its own process, so unnamed
// `serial` would not coordinate across files; but cargo's `--test-threads`
// default is per-binary, and SIGHUP from one binary cannot affect another
// process's children — what we observed empirically is that running these
// two suites in the same `cargo test` invocation interleaves them on the
// same OS process tree under macOS Grand Central Dispatch).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial_test::serial(sighup_signal)]
async fn events_tail_streams_agent_registered() {
    use tokio::io::AsyncBufReadExt;

    let daemon = TestDaemon::builder().require_auth(false).spawn().await;
    let home_dir = TempDir::new().expect("create temp home");
    write_client_config(&home_dir, &daemon);

    let home_path: PathBuf = home_dir.path().to_owned();

    // Spawn `trumpet events tail` as a child process.
    let trumpet_bin = assert_cmd::cargo::cargo_bin("trumpet");
    let mut child = tokio::process::Command::new(&trumpet_bin)
        .env("HOME", &home_path)
        .args(["events"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn events tail process");

    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = tokio::io::BufReader::new(stdout).lines();

    // Give the tail process time to spawn, connect to the daemon, and
    // subscribe to the SSE channel. The bus is broadcast-only — events
    // emitted *before* subscription are lost — so this delay must be
    // generous enough to win the race even under heavy CI load.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Register an agent to trigger an agent_registered event.
    trumpet_cmd(&home_dir)
        .args(["agent", "register", "--name", "event-test-agent"])
        .assert()
        .success();

    // Wait up to 5 s for an agent_registered line.
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match reader.next_line().await {
                Ok(Some(line)) if line.contains("agent_registered") => return true,
                Ok(Some(_)) => {}
                Ok(None) => return false,
                Err(_) => return false,
            }
        }
    })
    .await
    .unwrap_or(false);

    child.kill().await.ok();

    assert!(
        found,
        "events tail must print an agent_registered event within 5 seconds"
    );

    daemon
        .shutdown()
        .await
        .expect("daemon must shut down cleanly");
}
