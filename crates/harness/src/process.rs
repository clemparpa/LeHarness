//! Generic subprocess launcher for external tools: one JSON line in, one JSON
//! line out, over the child's stdin/stdout. Timeouts kill the child cleanly,
//! malformed responses are rejected fail-closed, and no secret value ever
//! reaches the child's env/argv (or the request line); tool output is masked
//! before being handed back to the caller.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::secrets::SecretStore;

/// Maximum accepted length of a single protocol line (1 MiO).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Default environment variable names the child may inherit from the harness
/// process. Anything else is stripped (`env_clear` before applying this list).
pub const DEFAULT_ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "LANG", "TERM", "TMPDIR"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolRequest {
    pub id: u64,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResponse {
    pub ok: bool,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("cannot spawn {binary}: {source}")]
    Spawn {
        binary: String,
        source: std::io::Error,
    },
    #[error("i/o error on tool process: {0}")]
    Io(#[from] std::io::Error),
    #[error("tool did not answer within {0:?}")]
    Timeout(Duration),
    #[error("malformed response line: {0}")]
    Malformed(String),
    #[error("response line exceeds {max} bytes")]
    LineTooLong { max: usize },
    #[error("a secret value would leak into the child process (binary/args/env/request)")]
    SecretLeak,
    #[error("tool process closed before answering")]
    Closed,
}

/// Parses one protocol line into a `ToolResponse`. Fail-closed: invalid JSON,
/// a missing or mistyped required field, or an unknown field is an error,
/// never a partially interpreted response.
pub fn parse_line(line: &str) -> Result<ToolResponse, ProcessError> {
    serde_json::from_str(line).map_err(|e| ProcessError::Malformed(e.to_string()))
}

/// A spawned tool process with a dedicated stdout reader thread.
pub struct Tool {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Result<String, ProcessError>>,
    secrets: Arc<SecretStore>,
}

impl Tool {
    /// Spawns `binary` with `args`, stdin/stdout piped. The child's environment
    /// is rebuilt from `env_allowlist` only (`env_clear` first). Fail-closed:
    /// if any known secret value appears in the binary path, the args, or a
    /// passed env value, `SecretLeak` is returned and nothing is spawned.
    pub fn spawn(
        binary: impl AsRef<Path>,
        args: &[String],
        secrets: &SecretStore,
        env_allowlist: &[&str],
    ) -> Result<Self, ProcessError> {
        let binary = binary.as_ref();
        let binary_str = binary.to_string_lossy().into_owned();
        let mut env_pairs: Vec<(String, String)> = Vec::new();
        for name in env_allowlist {
            if let Ok(value) = std::env::var(name) {
                env_pairs.push(((*name).to_string(), value));
            }
        }
        if secrets.contains_value(&binary_str)
            || args.iter().any(|a| secrets.contains_value(a))
            || env_pairs
                .iter()
                .any(|(k, v)| secrets.contains_value(k) || secrets.contains_value(v))
        {
            return Err(ProcessError::SecretLeak);
        }
        let mut cmd = Command::new(binary);
        cmd.args(args)
            .env_clear()
            .envs(env_pairs)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|source| ProcessError::Spawn {
            binary: binary_str,
            source,
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "no stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "no stdout pipe"))?;
        let (tx, rx) = channel::<Result<String, ProcessError>>();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = tx.send(Err(ProcessError::Closed));
                        break;
                    }
                    Ok(_) => {
                        if line.len() > MAX_LINE_BYTES {
                            let _ = tx.send(Err(ProcessError::LineTooLong {
                                max: MAX_LINE_BYTES,
                            }));
                            break;
                        }
                        if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(ProcessError::Io(e)));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            rx,
            secrets: Arc::new(secrets.clone()),
        })
    }

    /// Sends one request line and waits up to `timeout` for the response line.
    /// The response's `stdout`/`stderr` are masked with the secret store before
    /// being returned. On timeout or malformed response the child is killed.
    pub fn call(
        &mut self,
        req: &ToolRequest,
        timeout: Duration,
    ) -> Result<ToolResponse, ProcessError> {
        let line =
            serde_json::to_string(req).map_err(|e| ProcessError::Malformed(e.to_string()))?;
        if self.secrets.contains_value(&line) {
            return Err(ProcessError::SecretLeak);
        }
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let raw = match self.rx.recv_timeout(timeout) {
            Ok(Ok(line)) => line,
            Ok(Err(e)) => {
                self.kill_child();
                return Err(e);
            }
            Err(RecvTimeoutError::Timeout) => {
                self.kill_child();
                return Err(ProcessError::Timeout(timeout));
            }
            Err(RecvTimeoutError::Disconnected) => return Err(ProcessError::Closed),
        };
        match parse_line(&raw) {
            Ok(mut resp) => {
                resp.stdout = self.secrets.mask(&resp.stdout);
                resp.stderr = self.secrets.mask(&resp.stderr);
                Ok(resp)
            }
            Err(e) => {
                self.kill_child();
                Err(e)
            }
        }
    }

    /// True while the child process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Tool {
    fn drop(&mut self) {
        self.kill_child();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn store_with_secret() -> SecretStore {
        let mut store = SecretStore::new();
        store.insert("token", "s3cr3t-value", None);
        store
    }

    #[test]
    fn parse_line_accepts_valid_response() {
        let resp = parse_line(r#"{"ok":true,"code":0,"stdout":"hi","stderr":""}"#).unwrap();
        assert_eq!(
            resp,
            ToolResponse {
                ok: true,
                code: 0,
                stdout: "hi".into(),
                stderr: String::new(),
                truncated: false,
            }
        );
    }

    #[test]
    fn parse_line_accepts_optional_truncated_and_trailing_newline() {
        let resp = parse_line(
            "{\"ok\":false,\"code\":2,\"stdout\":\"\",\"stderr\":\"denied\",\"truncated\":true}\n",
        )
        .unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.code, 2);
        assert!(resp.truncated);
    }

    #[test]
    fn parse_line_rejects_malformed_inputs() {
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"ok":true,"code":0,"stdout":"x"}"#, // missing stderr
            r#"{"ok":"yes","code":0,"stdout":"","stderr":""}"#, // wrong type
            r#"{"ok":true,"code":0,"stdout":"","stderr":"","extra":1}"#, // unknown field
            "{\"ok\":true,\"code\":0,\"stdout\":\"a\nb\",\"stderr\":\"\"}", // raw newline in string
        ] {
            assert!(
                matches!(parse_line(bad), Err(ProcessError::Malformed(_))),
                "expected Malformed for {bad:?}"
            );
        }
    }

    #[test]
    fn spawn_refuses_secret_in_args() {
        let store = store_with_secret();
        let result = Tool::spawn(
            "/bin/true",
            &["s3cr3t-value".to_string()],
            &store,
            DEFAULT_ENV_ALLOWLIST,
        );
        assert!(matches!(result, Err(ProcessError::SecretLeak)));
    }

    #[test]
    fn spawn_refuses_secret_in_binary_path() {
        let store = store_with_secret();
        let result = Tool::spawn("/tmp/s3cr3t-value/tool", &[], &store, DEFAULT_ENV_ALLOWLIST);
        assert!(matches!(result, Err(ProcessError::SecretLeak)));
    }

    #[test]
    fn spawn_missing_binary_is_spawn_error() {
        let store = SecretStore::new();
        let result = Tool::spawn(
            "/nonexistent/le-harness-tool",
            &[],
            &store,
            DEFAULT_ENV_ALLOWLIST,
        );
        assert!(matches!(result, Err(ProcessError::Spawn { .. })));
    }

    proptest! {
        #[test]
        fn parse_line_is_fail_closed_roundtrip(
            ok in any::<bool>(),
            code in any::<i32>(),
            stdout in ".*",
            stderr in ".*",
            truncated in any::<bool>(),
        ) {
            let resp = ToolResponse { ok, code, stdout, stderr, truncated };
            let line = serde_json::to_string(&resp).unwrap();
            prop_assert_eq!(parse_line(&line).unwrap(), resp);
        }

        #[test]
        fn parse_line_never_partially_interprets_arbitrary_input(s in ".*") {
            if let Ok(resp) = parse_line(&s) {
                let line = serde_json::to_string(&resp).unwrap();
                prop_assert_eq!(parse_line(&line).unwrap(), resp);
            }
        }
    }
}
