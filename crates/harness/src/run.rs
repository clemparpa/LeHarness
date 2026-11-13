//! Local interactive CLI loop: `le-harness run --repo <path>` starts a session
//! on a local repo, reads the repo policy (`trusted.txt` / `sensitives.txt`),
//! and loops on stdin lines: `checkpoint`, `quit`/`exit`, EOF, and anything
//! else is forwarded to the configured tool through the subprocess launcher.
//! Everything printed goes through the secret mask. Fail-closed: no policy
//! file, no session.

use std::cell::RefCell;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use thiserror::Error;

use crate::policy::{PolicyError, RepoPolicy, SENSITIVES_FILE, TRUSTED_FILE};
use crate::process::{Tool, ToolRequest, DEFAULT_ENV_ALLOWLIST};
use crate::secrets::{SecretError, SecretStore};
use crate::session::{Persistence, PersistenceError, Session, SessionError, SessionId, Snapshot};

pub const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub repo: PathBuf,
    pub tool: Option<PathBuf>,
    pub secrets_file: Option<PathBuf>,
    pub tool_timeout: Duration,
}

impl RunConfig {
    pub fn new(repo: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            tool: None,
            secrets_file: None,
            tool_timeout: DEFAULT_TOOL_TIMEOUT,
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
    Process(#[from] crate::process::ProcessError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

/// In-memory persistence for the local CLI. File-based checkpoints and resume
/// are step 5; the core never touches the disk itself.
struct MemoryPersistence {
    saved: Rc<RefCell<Vec<Snapshot>>>,
}

impl Persistence for MemoryPersistence {
    fn save(&self, snapshot: &Snapshot) -> Result<(), PersistenceError> {
        self.saved.borrow_mut().push(snapshot.clone());
        Ok(())
    }

    fn load(&self, id: &SessionId) -> Result<Option<Snapshot>, PersistenceError> {
        Ok(self
            .saved
            .borrow()
            .iter()
            .rev()
            .find(|s| s.id == *id)
            .cloned())
    }
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

    let mut session = Session::start(
        SessionId::generate(),
        config.repo.clone(),
        NoHooks,
        MemoryPersistence {
            saved: Rc::new(RefCell::new(Vec::new())),
        },
    )?;
    writeln!(
        output,
        "session {} started on {} ({} trusted, {} sensitive patterns)",
        session.id(),
        config.repo.display(),
        policy.trusted.len(),
        policy.sensitives.len()
    )?;

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

    fn repo_with_policy(dir: &tempfile::TempDir) -> PathBuf {
        let root = dir.path().to_path_buf();
        std::fs::write(root.join(TRUSTED_FILE), "run-tests\n").unwrap();
        std::fs::write(root.join(SENSITIVES_FILE), ".*\\.pem\n").unwrap();
        root
    }

    fn run_str(config: &RunConfig, input: &str) -> Result<(RunOutcome, String), RunError> {
        let mut out = Vec::new();
        let outcome = run(config, Cursor::new(input.as_bytes().to_vec()), &mut out)?;
        Ok((outcome, String::from_utf8(out).unwrap()))
    }

    #[test]
    fn checkpoint_then_quit_runs_session_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let config = RunConfig::new(repo_with_policy(&dir));
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
        let config = RunConfig::new(repo_with_policy(&dir));
        let (outcome, out) = run_str(&config, "").unwrap();
        assert_eq!(outcome.checkpoints, 0);
        assert!(out.contains("started on"));
        assert!(out.contains("ended"));
    }

    #[test]
    fn blank_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let config = RunConfig::new(repo_with_policy(&dir));
        let (outcome, _) = run_str(&config, "\n   \nquit\n").unwrap();
        assert_eq!(outcome.tool_calls, 0);
        assert_eq!(outcome.checkpoints, 0);
    }

    #[test]
    fn tool_line_without_tool_is_an_error_and_the_loop_continues() {
        let dir = tempfile::tempdir().unwrap();
        let config = RunConfig::new(repo_with_policy(&dir));
        let (outcome, out) = run_str(&config, "echo hi\nquit\n").unwrap();
        assert_eq!(outcome.tool_calls, 1);
        assert!(out.contains("error: no tool configured"));
        assert!(out.contains("ended"));
    }

    #[test]
    fn missing_policy_file_aborts_before_any_session() {
        let dir = tempfile::tempdir().unwrap();
        let config = RunConfig::new(dir.path().to_path_buf());
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(result, Err(RunError::Policy(_))));
        assert!(out.is_empty());
    }

    #[test]
    fn missing_repo_aborts() {
        let dir = tempfile::tempdir().unwrap();
        let config = RunConfig::new(dir.path().join("nope"));
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(result, Err(RunError::Policy(_))));
    }

    #[test]
    fn missing_secrets_file_aborts_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = RunConfig::new(repo_with_policy(&dir));
        config.secrets_file = Some(dir.path().join("no-secrets.env"));
        let mut out = Vec::new();
        let result = run(&config, Cursor::new(Vec::new()), &mut out);
        assert!(matches!(result, Err(RunError::Secrets(_))));
    }
}
