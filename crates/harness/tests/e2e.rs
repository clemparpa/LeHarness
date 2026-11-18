//! E2e tests for the subprocess launcher against the committed dummy tool
//! binary (`CARGO_BIN_EXE_dummy-tool`).

use std::time::Duration;

use harness::process::{parse_line, Tool, ToolRequest, DEFAULT_ENV_ALLOWLIST};
use harness::secrets::SecretStore;

fn dummy_tool() -> &'static str {
    env!("CARGO_BIN_EXE_dummy-tool")
}

fn store_with_secret() -> SecretStore {
    let mut store = SecretStore::new();
    store.insert("token", "s3cr3t-value", None);
    store
}

fn spawn_dummy(store: &SecretStore) -> Tool {
    Tool::spawn(dummy_tool(), &[], store, DEFAULT_ENV_ALLOWLIST).unwrap()
}

#[test]
fn request_response_roundtrip() {
    let store = SecretStore::new();
    let mut tool = spawn_dummy(&store);
    let resp = tool
        .call(
            &ToolRequest {
                id: 1,
                input: "echo hello world".into(),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(resp.ok);
    assert_eq!(resp.code, 0);
    assert_eq!(resp.stdout, "hello world");
    assert!(tool.is_alive()); // the dummy keeps waiting for more lines
}

#[test]
fn tool_output_is_masked() {
    let store = store_with_secret();
    let mut tool = spawn_dummy(&store);
    let resp = tool
        .call(
            &ToolRequest {
                id: 1,
                input: "leak".into(),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(resp.stdout, "the token is ***, do not print it");
}

#[test]
fn child_env_excludes_non_allowlisted_vars() {
    std::env::set_var("LEHARNESS_E2E_SECRET", "s3cr3t-value");
    let store = store_with_secret();
    let mut tool = spawn_dummy(&store);
    let resp = tool
        .call(
            &ToolRequest {
                id: 1,
                input: "printenv".into(),
            },
            Duration::from_secs(5),
        )
        .unwrap();
    assert!(!resp.stdout.contains("s3cr3t-value"));
    assert!(!resp.stdout.contains("LEHARNESS_E2E_SECRET"));
}

#[test]
fn spawn_refuses_secret_in_passed_env_value() {
    std::env::set_var("LEHARNESS_E2E_SECRET", "s3cr3t-value");
    let store = store_with_secret();
    let result = Tool::spawn(dummy_tool(), &[], &store, &["PATH", "LEHARNESS_E2E_SECRET"]);
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::SecretLeak)
    ));
}

#[test]
fn malformed_response_is_rejected_and_child_killed() {
    let store = SecretStore::new();
    let mut tool = spawn_dummy(&store);
    let result = tool.call(
        &ToolRequest {
            id: 1,
            input: "garbage".into(),
        },
        Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::Malformed(_))
    ));
    assert!(!tool.is_alive());
}

#[test]
fn timeout_kills_the_child() {
    let store = SecretStore::new();
    let mut tool = spawn_dummy(&store);
    let result = tool.call(
        &ToolRequest {
            id: 1,
            input: "sleep".into(),
        },
        Duration::from_millis(300),
    );
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::Timeout(_))
    ));
    assert!(!tool.is_alive());
}

#[test]
fn request_containing_a_secret_is_refused() {
    let store = store_with_secret();
    let mut tool = spawn_dummy(&store);
    let result = tool.call(
        &ToolRequest {
            id: 1,
            input: "echo s3cr3t-value".into(),
        },
        Duration::from_secs(5),
    );
    assert!(matches!(
        result,
        Err(harness::process::ProcessError::SecretLeak)
    ));
}

#[test]
fn parse_line_is_exported_for_fuzzing() {
    let resp = parse_line(r#"{"ok":true,"code":0,"stdout":"x","stderr":""}"#).unwrap();
    assert_eq!(resp.stdout, "x");
}

// --- Step 4: local CLI (`le-harness run --repo ...`) ---

fn harness_bin() -> &'static str {
    env!("CARGO_BIN_EXE_le-harness")
}

fn repo_with_policy(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("trusted.txt"), "run-tests\n").unwrap();
    std::fs::write(root.join("sensitives.txt"), ".*\\.pem\n").unwrap();
    root
}

fn run_cli(args: &[&str], stdin_data: &str) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(harness_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_data.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn cli_scripted_session_with_tool_and_masking() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let secrets = dir.path().join("secrets.env");
    std::fs::write(&secrets, "token=s3cr3t-value\n").unwrap();
    let tool = dummy_tool();
    let root_arg = root.to_str().unwrap().to_string();
    let secrets_arg = secrets.to_str().unwrap().to_string();
    let output = run_cli(
        &[
            "run",
            "--repo",
            &root_arg,
            "--tool",
            tool,
            "--secrets-file",
            &secrets_arg,
        ],
        "checkpoint\necho hello world\nleak\nquit\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("started on"), "stdout: {stdout}");
    assert!(
        stdout.contains("1 trusted, 1 sensitive patterns"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("checkpoint 1"), "stdout: {stdout}");
    assert!(stdout.contains("hello world"), "stdout: {stdout}");
    assert!(stdout.contains("the token is ***"), "stdout: {stdout}");
    assert!(!stdout.contains("s3cr3t-value"), "stdout: {stdout}");
    assert!(stdout.contains("ended"), "stdout: {stdout}");
}

#[test]
fn cli_eof_ends_session_with_success() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let root_arg = root.to_str().unwrap().to_string();
    let output = run_cli(&["run", "--repo", &root_arg], "checkpoint\n");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("checkpoint 1"));
    assert!(stdout.contains("ended"));
}

#[test]
fn cli_without_tool_reports_error_and_continues() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let root_arg = root.to_str().unwrap().to_string();
    let output = run_cli(&["run", "--repo", &root_arg], "echo hi\nquit\n");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("error: no tool configured"),
        "stdout: {stdout}"
    );
}

#[test]
fn cli_missing_policy_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let root_arg = dir.path().to_str().unwrap().to_string();
    let output = run_cli(&["run", "--repo", &root_arg], "");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("trusted.txt"), "stderr: {stderr}");
}

#[test]
fn cli_missing_repo_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let root_arg = dir.path().join("nope").to_str().unwrap().to_string();
    let output = run_cli(&["run", "--repo", &root_arg], "");
    assert_eq!(output.status.code(), Some(2));
}

// --- Step 5: file checkpoints + resume (`--state-dir`, `--resume`) ---

fn session_id_from_stdout(stdout: &str) -> String {
    stdout
        .split_whitespace()
        .find(|t| t.starts_with("sess-"))
        .expect("session id in stdout")
        .to_string()
}

/// Spawns the CLI, feeds it lines, then kills the child mid-session (no `quit`,
/// no EOF): simulates a crash, the session stays `Running` on disk.
fn run_cli_killed(args: &[&str], stdin_data: &str) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(harness_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(stdin_data.as_bytes())
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    child.kill().unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn cli_checkpoint_persists_a_json_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let state = dir.path().join("state");
    let root_arg = root.to_str().unwrap().to_string();
    let state_arg = state.to_str().unwrap().to_string();
    let output = run_cli(
        &["run", "--repo", &root_arg, "--state-dir", &state_arg],
        "checkpoint\nquit\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let id = session_id_from_stdout(&stdout);
    let raw = std::fs::read_to_string(state.join(format!("{id}.json"))).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["id"], id);
    assert_eq!(v["checkpoints"], 1);
    assert_eq!(v["state"], "Ended");
}

#[test]
fn cli_resume_after_a_crash_continues_the_checkpoint_counter() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let state = dir.path().join("state");
    let root_arg = root.to_str().unwrap().to_string();
    let state_arg = state.to_str().unwrap().to_string();

    let crashed = run_cli_killed(
        &["run", "--repo", &root_arg, "--state-dir", &state_arg],
        "checkpoint\ncheckpoint\n",
    );
    let stdout = String::from_utf8(crashed.stdout).unwrap();
    assert!(stdout.contains("checkpoint 2"), "stdout: {stdout}");
    let id = session_id_from_stdout(&stdout);
    let raw = std::fs::read_to_string(state.join(format!("{id}.json"))).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["checkpoints"], 2);
    assert_eq!(v["state"], "Running");

    let output = run_cli(
        &[
            "run",
            "--repo",
            &root_arg,
            "--state-dir",
            &state_arg,
            "--resume",
            &id,
        ],
        "checkpoint\nquit\n",
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("resumed on"), "stdout: {stdout}");
    assert!(stdout.contains("2 checkpoints"), "stdout: {stdout}");
    assert!(stdout.contains("checkpoint 3"), "stdout: {stdout}");
    assert!(stdout.contains("ended"), "stdout: {stdout}");
}

#[test]
fn cli_resume_unknown_id_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let state = dir.path().join("state");
    let root_arg = root.to_str().unwrap().to_string();
    let state_arg = state.to_str().unwrap().to_string();
    let output = run_cli(
        &[
            "run",
            "--repo",
            &root_arg,
            "--state-dir",
            &state_arg,
            "--resume",
            "sess-nope-0",
        ],
        "",
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not found"), "stderr: {stderr}");
}

#[test]
fn cli_resume_ended_session_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let state = dir.path().join("state");
    let root_arg = root.to_str().unwrap().to_string();
    let state_arg = state.to_str().unwrap().to_string();
    let first = run_cli(
        &["run", "--repo", &root_arg, "--state-dir", &state_arg],
        "quit\n",
    );
    assert_eq!(first.status.code(), Some(0));
    let id = session_id_from_stdout(&String::from_utf8(first.stdout).unwrap());
    let output = run_cli(
        &[
            "run",
            "--repo",
            &root_arg,
            "--state-dir",
            &state_arg,
            "--resume",
            &id,
        ],
        "",
    );
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("already ended"), "stderr: {stderr}");
}
