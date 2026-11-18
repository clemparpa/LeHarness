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

// --- Step 8: multi-session orchestrator (`le-harness serve`) ---

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

use harness::serve::{Orchestrator, Response, ServeConfig};

fn repos_dir_with(dir: &tempfile::TempDir, names: &[&str]) -> PathBuf {
    let repos = dir.path().join("repos");
    for name in names {
        let repo = repos.join(name);
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join("trusted.txt"), "run-tests\n").unwrap();
        std::fs::write(repo.join("sensitives.txt"), ".*\\.pem\n").unwrap();
    }
    repos
}

fn serve_config(
    dir: &tempfile::TempDir,
    repos: &std::path::Path,
    harness_bin: &str,
) -> ServeConfig {
    let mut c = ServeConfig::new(repos.to_path_buf(), dir.path().join("state"));
    c.harness_bin = PathBuf::from(harness_bin);
    c
}

/// A spawned `le-harness serve` process with a line-JSON client on its
/// stdin/stdout (reader thread -> channel, so reads can time out).
struct ServeClient {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

impl ServeClient {
    fn spawn(args: &[String]) -> Self {
        let mut child = Command::new(harness_bin())
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel::<String>();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self { child, stdin, rx }
    }

    fn send(&mut self, line: &str) {
        self.stdin.write_all(line.as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self, timeout: Duration) -> serde_json::Value {
        let line = self
            .rx
            .recv_timeout(timeout)
            .expect("a response line within the timeout");
        serde_json::from_str(line.trim()).unwrap()
    }

    fn wait(mut self) -> std::process::ExitStatus {
        self.child.wait().unwrap()
    }
}

fn serve_args(repos: &std::path::Path, state: &std::path::Path, extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        "serve".to_string(),
        "--repos-dir".to_string(),
        repos.to_str().unwrap().to_string(),
        "--tool".to_string(),
        dummy_tool().to_string(),
        "--state-dir".to_string(),
        state.to_str().unwrap().to_string(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

#[test]
fn serve_handles_n_sessions_concurrently_measured_at_wall_clock() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let state = dir.path().join("state");
    let args = serve_args(&repos, &state, &["--max-sessions", "8"]);
    let mut client = ServeClient::spawn(&args);

    let mut ids = Vec::new();
    for _ in 0..4 {
        client.send(r#"{"op":"start","repo":"r1"}"#);
        let resp = client.recv(Duration::from_secs(10));
        assert_eq!(resp["op"], "started", "resp: {resp}");
        ids.push(resp["session"].as_str().unwrap().to_string());
    }
    assert_eq!(ids.len(), 4);

    let t0 = Instant::now();
    for id in &ids {
        client.send(&format!(
            r#"{{"op":"call","session":"{id}","input":"sleep-ms 700"}}"#
        ));
        let ack = client.recv(Duration::from_secs(5));
        assert_eq!(ack["op"], "accepted", "ack: {ack}");
        assert_eq!(ack["session"], *id);
    }
    for _ in 0..4 {
        let r = client.recv(Duration::from_secs(10));
        assert_eq!(r["op"], "result", "r: {r}");
        assert_eq!(r["ok"], true);
        assert_eq!(r["stdout"], "sleep-ms 700");
    }
    let elapsed = t0.elapsed();
    // sequential would take >= 4 x 700 ms = 2.8 s; concurrent ~= 700 ms
    assert!(
        elapsed < Duration::from_secs(2),
        "no concurrency: {elapsed:?}"
    );

    client.send(r#"{"op":"quit"}"#);
    let end = client.recv(Duration::from_secs(5));
    assert_eq!(end["op"], "ended");
    let status = client.wait();
    assert_eq!(status.code(), Some(0));
}

#[test]
fn serve_allows_two_sessions_on_two_repos_no_worktree_lock() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1", "r2"]);
    let state = dir.path().join("state");
    let mut client = ServeClient::spawn(&serve_args(&repos, &state, &[]));
    for repo in ["r1", "r2"] {
        client.send(&format!(r#"{{"op":"start","repo":"{repo}"}}"#));
        assert_eq!(client.recv(Duration::from_secs(10))["op"], "started");
    }
    client.send(r#"{"op":"list"}"#);
    let list = client.recv(Duration::from_secs(5));
    assert_eq!(list["op"], "sessions");
    assert_eq!(list["sessions"].as_array().unwrap().len(), 2);
    client.send(r#"{"op":"quit"}"#);
    assert_eq!(client.recv(Duration::from_secs(5))["op"], "ended");
    assert_eq!(client.wait().code(), Some(0));
}

#[test]
fn serve_refuses_beyond_max_sessions_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let state = dir.path().join("state");
    let mut client = ServeClient::spawn(&serve_args(&repos, &state, &["--max-sessions", "2"]));
    for _ in 0..2 {
        client.send(r#"{"op":"start","repo":"r1"}"#);
        assert_eq!(client.recv(Duration::from_secs(10))["op"], "started");
    }
    client.send(r#"{"op":"start","repo":"r1"}"#);
    let resp = client.recv(Duration::from_secs(5));
    assert_eq!(resp["op"], "error", "resp: {resp}");
    assert!(resp["error"].as_str().unwrap().contains("max-sessions"));
    client.send(r#"{"op":"bounds"}"#);
    let b = client.recv(Duration::from_secs(5));
    assert_eq!(b["op"], "bounds");
    assert_eq!(b["max_sessions"], 2);
    assert_eq!(b["active"], 2);
    client.send(r#"{"op":"quit"}"#);
    client.wait();
}

#[test]
fn serve_rejects_invalid_repo_names() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let state = dir.path().join("state");
    let mut client = ServeClient::spawn(&serve_args(&repos, &state, &[]));
    for bad in ["..", "a/b", "no-such"] {
        client.send(&format!(r#"{{"op":"start","repo":"{bad}"}}"#));
        let resp = client.recv(Duration::from_secs(5));
        assert_eq!(resp["op"], "error", "repo {bad}: {resp}");
    }
    client.send(r#"{"op":"quit"}"#);
    client.wait();
}

#[test]
fn serve_malformed_request_line_gets_error_and_loop_continues() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let state = dir.path().join("state");
    let mut client = ServeClient::spawn(&serve_args(&repos, &state, &[]));
    client.send("garbage");
    let resp = client.recv(Duration::from_secs(5));
    assert_eq!(resp["op"], "error");
    client.send(r#"{"op":"bounds"}"#);
    assert_eq!(client.recv(Duration::from_secs(5))["op"], "bounds");
    client.send(r#"{"op":"quit"}"#);
    client.wait();
}

#[test]
fn serve_session_timeout_kills_the_session_and_reports_error() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let state = dir.path().join("state");
    let mut client = ServeClient::spawn(&serve_args(&repos, &state, &["--session-timeout", "1"]));
    client.send(r#"{"op":"start","repo":"r1"}"#);
    let started = client.recv(Duration::from_secs(10));
    let id = started["session"].as_str().unwrap().to_string();
    let t0 = Instant::now();
    client.send(&format!(
        r#"{{"op":"call","session":"{id}","input":"sleep"}}"#
    ));
    assert_eq!(client.recv(Duration::from_secs(5))["op"], "accepted");
    let resp = client.recv(Duration::from_secs(10));
    assert_eq!(resp["op"], "error", "resp: {resp}");
    assert!(resp["error"].as_str().unwrap().contains("timeout"));
    assert_eq!(resp["session"], id);
    assert!(t0.elapsed() < Duration::from_secs(5));
    client.send(r#"{"op":"list"}"#);
    let list = client.recv(Duration::from_secs(5));
    assert_eq!(list["sessions"].as_array().unwrap().len(), 0);
    client.send(r#"{"op":"quit"}"#);
    assert_eq!(client.wait().code(), Some(0));
}

#[test]
fn serve_kill_then_quit_leaves_no_child_process_linux() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let state = dir.path().join("state");
    let mut client = ServeClient::spawn(&serve_args(&repos, &state, &[]));
    client.send(r#"{"op":"start","repo":"r1"}"#);
    let started = client.recv(Duration::from_secs(10));
    let id = started["session"].as_str().unwrap().to_string();
    let serve_pid = client.child.id();
    client.send(&format!(r#"{{"op":"kill","session":"{id}"}}"#));
    assert_eq!(client.recv(Duration::from_secs(5))["op"], "killed");
    client.send(r#"{"op":"quit"}"#);
    assert_eq!(client.recv(Duration::from_secs(5))["op"], "ended");
    assert_eq!(client.wait().code(), Some(0));
    #[cfg(target_os = "linux")]
    {
        // no leftover `le-harness run --json` child of the serve process
        let mut leftovers = Vec::new();
        for entry in std::fs::read_dir("/proc").unwrap() {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let pid: u32 = name.parse().unwrap();
            let Ok(cmdline) = std::fs::read_to_string(format!("/proc/{pid}/cmdline")) else {
                continue;
            };
            if !cmdline.contains("run") || !cmdline.contains("--json") {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // field 4 (1-indexed) of stat is ppid
            let ppid: u32 = stat
                .rsplit(')')
                .next()
                .and_then(|rest| rest.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if ppid == serve_pid {
                leftovers.push(pid);
            }
        }
        assert!(
            leftovers.is_empty(),
            "leftover session processes: {leftovers:?}"
        );
    }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn orchestrator_start_call_result_against_dummy_session_process() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let mut config = serve_config(&dir, &repos, dummy_tool());
    config.session_timeout = Duration::from_secs(5);
    let mut orch = Orchestrator::new(&config).unwrap();
    let resps = orch.handle_line(r#"{"op":"start","repo":"r1"}"#);
    assert!(
        matches!(&resps[0], Response::Started { .. }),
        "{:?}",
        resps[0]
    );
    let sid = match &resps[0] {
        Response::Started { session, .. } => session.clone(),
        _ => unreachable!(),
    };
    let resps = orch.handle_line(&format!(
        r#"{{"op":"call","session":"{sid}","input":"echo hello"}}"#
    ));
    assert!(matches!(&resps[0], Response::Accepted { .. }));
    let events = orch.collect_events(Duration::from_secs(5));
    let result = events
        .iter()
        .find(|r| matches!(r, Response::Result { .. }))
        .expect("a Result event");
    match result {
        Response::Result {
            session,
            ok,
            stdout,
            ..
        } => {
            assert_eq!(session, &sid);
            assert!(ok);
            assert_eq!(stdout, "hello");
        }
        _ => unreachable!(),
    }
    assert_eq!(orch.active_count(), 1);
}

#[test]
fn orchestrator_malformed_session_response_is_fail_closed_and_kills() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let mut config = serve_config(&dir, &repos, dummy_tool());
    config.session_timeout = Duration::from_secs(5);
    let mut orch = Orchestrator::new(&config).unwrap();
    let resps = orch.handle_line(r#"{"op":"start","repo":"r1","id":"sess-m1"}"#);
    assert!(matches!(&resps[0], Response::Started { .. }));
    let pid = orch.child_pid("sess-m1").unwrap();
    orch.handle_line(r#"{"op":"call","session":"sess-m1","input":"garbage"}"#);
    let events = orch.collect_events(Duration::from_secs(5));
    assert!(
        events.iter().any(|r| matches!(
            r,
            Response::Error { error, .. } if error.contains("malformed")
        )),
        "events: {events:?}"
    );
    assert_eq!(orch.active_count(), 0);
    #[cfg(unix)]
    assert!(!pid_alive(pid), "session child must be killed");
}

#[test]
fn orchestrator_timeout_kills_a_silent_session() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let mut config = serve_config(&dir, &repos, dummy_tool());
    config.session_timeout = Duration::from_millis(300);
    let mut orch = Orchestrator::new(&config).unwrap();
    let resps = orch.handle_line(r#"{"op":"start","repo":"r1","id":"sess-t1"}"#);
    assert!(matches!(&resps[0], Response::Started { .. }));
    let pid = orch.child_pid("sess-t1").unwrap();
    orch.handle_line(r#"{"op":"call","session":"sess-t1","input":"sleep"}"#);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut events = Vec::new();
    while Instant::now() < deadline {
        events.extend(orch.collect_events(Duration::from_millis(50)));
        if events
            .iter()
            .any(|r| matches!(r, Response::Error { error, .. } if error.contains("timeout")))
        {
            break;
        }
    }
    assert!(
        events
            .iter()
            .any(|r| matches!(r, Response::Error { error, .. } if error.contains("timeout"))),
        "events: {events:?}"
    );
    assert_eq!(orch.active_count(), 0);
    #[cfg(unix)]
    assert!(!pid_alive(pid), "timed-out session child must be killed");
}

#[test]
fn orchestrator_audit_log_never_contains_request_or_response_content() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let mut config = serve_config(&dir, &repos, dummy_tool());
    config.session_timeout = Duration::from_secs(5);
    let mut orch = Orchestrator::new(&config).unwrap();
    orch.handle_line(r#"{"op":"start","repo":"r1","id":"sess-a1"}"#);
    orch.handle_line(r#"{"op":"call","session":"sess-a1","input":"echo secret-content-xyz"}"#);
    let _ = orch.collect_events(Duration::from_secs(5));
    let audit = std::fs::read_to_string(dir.path().join("state/audit/sess-a1.jsonl")).unwrap();
    assert!(audit.contains("start repo=r1"), "audit: {audit}");
    assert!(audit.contains("accepted req=1"), "audit: {audit}");
    assert!(audit.contains("result req=1"), "audit: {audit}");
    assert!(!audit.contains("secret-content-xyz"), "audit: {audit}");
    assert!(!audit.contains("hello"), "audit: {audit}");
}

#[test]
fn orchestrator_max_sessions_refusal_with_real_spawns() {
    let dir = tempfile::tempdir().unwrap();
    let repos = repos_dir_with(&dir, &["r1"]);
    let mut config = serve_config(&dir, &repos, dummy_tool());
    config.max_sessions = 1;
    let mut orch = Orchestrator::new(&config).unwrap();
    assert!(matches!(
        orch.handle_line(r#"{"op":"start","repo":"r1"}"#)[0],
        Response::Started { .. }
    ));
    let resps = orch.handle_line(r#"{"op":"start","repo":"r1"}"#);
    assert!(matches!(&resps[0], Response::Error { error, .. } if error.contains("max-sessions")));
    assert_eq!(orch.active_count(), 1);
}

proptest::proptest! {
    #[test]
    fn no_session_child_process_survives_orchestrator_drop(n in 0usize..6) {
        let dir = tempfile::tempdir().unwrap();
        let repos = repos_dir_with(&dir, &["r1"]);
        let mut config = serve_config(&dir, &repos, dummy_tool());
        config.max_sessions = 8;
        config.session_timeout = Duration::from_secs(5);
        let mut orch = Orchestrator::new(&config).unwrap();
        let mut pids = Vec::new();
        for i in 0..n {
            let resps = orch.handle_line(&format!(r#"{{"op":"start","repo":"r1","id":"sess-p{i}"}}"#));
            if let Response::Started { session, .. } = &resps[0] {
                pids.push(orch.child_pid(session).unwrap());
            } else {
                panic!("start failed: {:?}", resps[0]);
            }
        }
        drop(orch);
        #[cfg(unix)]
        for pid in pids {
            proptest::prop_assert!(!pid_alive(pid), "child {pid} still alive after drop");
        }
    }
}

#[test]
fn cli_run_json_mode_speaks_line_json_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let root = repo_with_policy(&dir);
    let state = dir.path().join("state");
    let root_arg = root.to_str().unwrap().to_string();
    let state_arg = state.to_str().unwrap().to_string();
    let req = |id: u64, input: &str| {
        serde_json::to_string(&serde_json::json!({"id": id, "input": input})).unwrap()
    };
    let output = run_cli(
        &[
            "run",
            "--repo",
            &root_arg,
            "--tool",
            dummy_tool(),
            "--state-dir",
            &state_arg,
            "--json",
        ],
        &format!(
            "{}\n{}\n{}\n",
            req(1, "status"),
            req(2, "echo hello-json"),
            req(3, "quit")
        ),
    );
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "stdout: {stdout}");
    let r1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(r1["ok"], true);
    assert!(r1["stdout"].as_str().unwrap().contains("sess-"));
    let r2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(r2["ok"], true);
    assert_eq!(r2["stdout"], "hello-json");
    let r3: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(r3["ok"], true);
    assert!(r3["stdout"].as_str().unwrap().contains("ended"));
}
