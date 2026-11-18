//! Local interactive CLI loop: `le-harness run --repo <path>` starts a session
//! on a local repo (or resumes one with `--resume <id>`, step 5), reads the
//! repo policy (`trusted.txt` / `sensitives.txt`), and loops on stdin lines:
//! `checkpoint`, `quit`/`exit`, EOF, and anything else is forwarded to the
//! configured tool through the subprocess launcher. Snapshots are persisted
//! to disk (JSON, one file per session) so a checkpoint survives a restart.
//! Everything printed goes through the secret mask. Fail-closed: no policy
//! file, no session.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

use crate::persistence::FilePersistence;
use crate::policy::{PolicyError, RepoPolicy, SENSITIVES_FILE, TRUSTED_FILE};
use crate::process::{Tool, ToolRequest, DEFAULT_ENV_ALLOWLIST};
use crate::secrets::{SecretError, SecretStore};
use crate::session::{PersistenceError, Session, SessionError, SessionId};

pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub repo: PathBuf,
    pub tool: Option<PathBuf>,
    pub secrets_file: Option<PathBuf>,
    pub tool_timeout: Duration,
    pub state_dir: Option<PathBuf>,
    pub resume: Option<SessionId>,
}

impl RunConfig {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            tool: None,
            secrets_file: None,
            tool_timeout: DEFAULT_TOOL_TIMEOUT,
            state_dir: None,
            resume: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunOutcome {
    pub checkpoints: u64,
    pub tool_calls: u64,
}

#[derive(Debug, Error)]
pub enum RunError {
    #[error(transparent)]
    Policy(#[from] PolicyError),
    #[error(transparent)]
    Secrets(#[from] SecretError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Process(#[from] crate::process::ProcessError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

struct NoHooks;

impl crate::session::Hooks for NoHooks {}

/// Runs one interactive session. `input` is read line by line until
/// `quit`/`exit` or EOF; `output` receives the session events and tool
/// responses (masked).
pub fn run(
    config: &RunConfig,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<RunOutcome, RunError> {
    let policy = RepoPolicy::load(&config.repo)?;
    let secrets = match &config.secrets_file {
        Some(path) => SecretStore::from_file(path)?,
        None => SecretStore::new(),
    };

    let store = match &config.state_dir {
        Some(dir) => FilePersistence::new(dir)?,
        None => FilePersistence::new(FilePersistence::default_dir()?)?,
    };
    let mut session = match &config.resume {
        Some(id) => Session::resume(id.clone(), config.repo.clone(), NoHooks, store)?,
        None => Session::start(SessionId::generate(), config.repo.clone(), NoHooks, store)?,
    };
    match &config.resume {
        Some(_) => writeln!(
            output,
            "session {} resumed on {} ({} checkpoints, {} trusted, {} sensitive patterns)",
            session.id(),
            config.repo.display(),
            session.checkpoints(),
            policy.trusted.len(),
            policy.sensitives.len()
        )?,
        None => writeln!(
            output,
            "session {} started on {} ({} trusted, {} sensitive patterns)",
            session.id(),
            config.repo.display(),
            policy.trusted.len(),
            policy.sensitives.len()
        )?,
    }

    let mut tool = match &config.tool {
        Some(binary) => Some(Tool::spawn(
            binary,
            &[
                "--trusted-file".to_string(),
                config
                    .repo
                    .join(TRUSTED_FILE)
                    .to_string_lossy()
                    .into_owned(),
                "--sensitives-file".to_string(),
                config
                    .repo
                    .join(SENSITIVES_FILE)
                    .to_string_lossy()
                    .into_owned(),
            ],
            &secrets,
            DEFAULT_ENV_ALLOWLIST,
        )?),
        None => None,
    };

    let mut outcome = RunOutcome {
        checkpoints: 0,
        tool_calls: 0,
    };
    for line in input.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line {
            "quit" | "exit" => break,
            "checkpoint" => {
                outcome.checkpoints = session.checkpoint()?;
                writeln!(output, "checkpoint {}", outcome.checkpoints)?;
            }
            _ => {
                outcome.tool_calls += 1;
                match &mut tool {
                    Some(tool) => {
                        let req = ToolRequest {
                            id: outcome.tool_calls,
                            input: line.to_string(),
                        };
                        match tool.call(&req, config.tool_timeout) {
                            Ok(resp) => {
                                writeln!(output, "ok {} code {}", resp.ok, resp.code)?;
                                let stdout = secrets.mask(&resp.stdout);
                                if !stdout.is_empty() {
                                    write!(output, "{stdout}")?;
                                    if !stdout.ends_with('\n') {
                                        writeln!(output)?;
                                    }
                                }
                                let stderr = secrets.mask(&resp.stderr);
                                if !stderr.is_empty() {
                                    write!(output, "{stderr}")?;
                                    if !stderr.ends_with('\n') {
                                        writeln!(output)?;
                                    }
                                }
                            }
                            Err(e) => {
                                writeln!(output, "error: {e}")?;
                            }
                        }
                    }
                    None => {
                        writeln!(output, "error: no tool configured")?;
                    }
                }
            }
        }
    }
    session.end()?;
    writeln!(output, "session {} ended", session.id())?;
    output.flush()?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::str::FromStr;

    fn repo_with_policy(dir: &tempfile::TempDir) -> PathBuf {
        let root = dir.path().to_path_buf();
        std::fs::write(root.join(TRUSTED_FILE), "run-tests\n").unwrap();
        std::fs::write(root.join(SENSITIVES_FILE), ".*\\.pem\n").unwrap();
        root
    }

    fn config(dir: &tempfile::TempDir) -> RunConfig {
        let mut config = RunConfig::new(repo_with_policy(dir));
        config.state_dir = Some(dir.path().join("state"));
        config
    }

    fn run_str(config: &RunConfig, input: &str) -> Result<(RunOutcome, String), RunError> {
        let mut out = Vec::new();
        let outcome = run(config, Cursor::new(input.as_bytes().to_vec()), &mut out)?;
        Ok((outcome, String::from_utf8(out).unwrap()))
    }

    fn session_id_from_output(out: &str) -> SessionId {
        let token = out
            .split_whitespace()
            .find(|t| t.starts_with("sess-"))
            .expect("session id in output");
        SessionId::from_str(token).unwrap()
    }

    #[test]
    fn checkpoint_then_quit_runs_session_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        let (outcome, out) = run_str(&config, "checkpoint\ncheckpoint\nquit\n").unwrap();
        assert_eq!(outcome.checkpoints, 2);
        assert_eq!(outcome.tool_calls, 0);
        assert!(out.contains("started on"));
        assert!(out.contains("checkpoint 1"));
        assert!(out.contains("checkpoint 2"));
        assert!(out.contains("ended"));
    }

    #[test]
    fn eof_ends_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        let (outcome, out) = run_str(&config, "").unwrap();
        assert_eq!(outcome.checkpoints, 0);
        assert!(out.contains("started on"));
        assert!(out.contains("ended"));
    }

    #[test]
    fn blank_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        let (outcome, _) = run_str(&config, "\n   \nquit\n").unwrap();
        assert_eq!(outcome.tool_calls, 0);
        assert_eq!(outcome.checkpoints, 0);
    }

    #[test]
    fn tool_line_without_tool_is_an_error_and_the_loop_continues() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        let (outcome, out) = run_str(&config, "echo hi\nquit\n").unwrap();
        assert_eq!(outcome.tool_calls, 1);
        assert!(out.contains("error: no tool configured"));
        assert!(out.contains("ended"));
    }

    #[test]
    fn missing_policy_file_aborts_before_any_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = RunConfig::new(dir.path().to_path_buf());
        config.state_dir = Some(dir.path().join("state"));
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(result, Err(RunError::Policy(_))));
        assert!(out.is_empty());
    }

    #[test]
    fn missing_repo_aborts() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = RunConfig::new(dir.path().join("nope"));
        config.state_dir = Some(dir.path().join("state"));
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(result, Err(RunError::Policy(_))));
    }

    #[test]
    fn missing_secrets_file_aborts_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(&dir);
        config.secrets_file = Some(dir.path().join("no-secrets.env"));
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(result, Err(RunError::Secrets(_))));
    }

    #[test]
    fn checkpoint_persists_a_json_file_in_the_state_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        let (_, out) = run_str(&config, "checkpoint\nquit\n").unwrap();
        let id = session_id_from_output(&out);
        let file = dir.path().join("state").join(format!("{id}.json"));
        let raw = std::fs::read_to_string(file).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["id"], id.to_string());
        assert_eq!(v["checkpoints"], 1);
        assert_eq!(v["state"], "Ended");
    }

    /// Simulates a crashed process: a session is started and checkpointed on
    /// the given state dir, then dropped without `end()` (kill -9, OOM…).
    fn crashed_session(dir: &tempfile::TempDir, checkpoints: u64) -> (PathBuf, SessionId) {
        let root = repo_with_policy(dir);
        let state = dir.path().join("state");
        let store = FilePersistence::new(&state).unwrap();
        let mut session =
            Session::start(SessionId::generate(), root.clone(), NoHooks, store).unwrap();
        for _ in 0..checkpoints {
            session.checkpoint().unwrap();
        }
        let id = session.id().clone();
        drop(session);
        (state, id)
    }

    #[test]
    fn stop_then_resume_restores_checkpoints_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = crashed_session(&dir, 2);

        let mut config = RunConfig::new(repo_with_policy(&dir));
        config.state_dir = Some(state.clone());
        config.resume = Some(id.clone());
        let (outcome, out) = run_str(&config, "checkpoint\nquit\n").unwrap();
        assert_eq!(outcome.checkpoints, 3);
        assert!(out.contains("resumed on"), "out: {out}");
        assert!(out.contains("2 checkpoints"), "out: {out}");
        assert!(out.contains("checkpoint 3"), "out: {out}");
        assert!(out.contains("ended"), "out: {out}");

        let raw = std::fs::read_to_string(state.join(format!("{id}.json"))).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["checkpoints"], 3);
        assert_eq!(v["state"], "Ended");
    }

    #[test]
    fn resume_unknown_id_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config(&dir);
        config.resume = Some(SessionId::from_str("sess-nope-0").unwrap());
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(
            result,
            Err(RunError::Session(SessionError::NotFound(_)))
        ));
        assert!(out.is_empty());
    }

    #[test]
    fn resume_ended_session_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let config = config(&dir);
        let (_, out) = run_str(&config, "quit\n").unwrap();
        let id = session_id_from_output(&out);

        let mut resumed = config.clone();
        resumed.resume = Some(id.clone());
        let mut out = Vec::new();
        let result = run(&resumed, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(
            result,
            Err(RunError::Session(SessionError::AlreadyEnded(ref eid))) if *eid == id
        ));
    }

    #[test]
    fn resume_with_a_different_root_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (state, id) = crashed_session(&dir, 0);

        let other = tempfile::tempdir().unwrap();
        let mut resumed = RunConfig::new(repo_with_policy(&other));
        resumed.state_dir = Some(state);
        resumed.resume = Some(id);
        let mut out = Vec::new();
        let result = run(&resumed, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(
            result,
            Err(RunError::Session(SessionError::RootMismatch { .. }))
        ));
    }
}
