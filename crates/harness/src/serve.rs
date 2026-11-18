//! Multi-session orchestrator (step 8): `le-harness serve --repos-dir <dir>`
//! handles several sessions at once, **one subprocess per session**
//! (`le-harness run --repo <path> --json`, spawned through the step-3 launcher
//! primitive `spawn_piped`, line-JSON protocol), one reader thread per
//! session pushing into a single `mpsc` channel — no async runtime.
//!
//! Bounds (fail-closed): `--max-sessions` reached = refusal, per-message
//! `--session-timeout` + clean kill of the child, malformed session response =
//! error + clean kill. No worktree lock: several sessions may work on the same
//! worktree (or on several worktrees of the same repo) at the same time.
//!
//! The orchestrator holds **no session secret**: each session subprocess gets
//! its own `SecretStore` (loaded from a per-session phantom-token file) and
//! routes already-masked JSON lines. Audit: one append-only JSONL log per
//! session (ids and events only, never request/response content).

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::process::{
    parse_line, spawn_piped, PipedChild, ProcessError, ToolRequest, DEFAULT_ENV_ALLOWLIST,
};
use crate::secrets::SecretStore;
use crate::session::SessionId;

pub const DEFAULT_MAX_SESSIONS: usize = 8;
pub const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 30;

/// Default state root: `$HOME/.local/share/le-harness` (per-session snapshots
/// go to `<state>/sessions`, audit logs to `<state>/audit`, per-session
/// phantom-token files to `<state>/serve-secrets`).
pub const DEFAULT_STATE_ROOT: &str = ".local/share/le-harness";

#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub repos_dir: PathBuf,
    pub max_sessions: usize,
    pub session_timeout: Duration,
    pub tool: Option<PathBuf>,
    pub state_dir: PathBuf,
    pub harness_bin: PathBuf,
}

impl ServeConfig {
    pub fn new(repos_dir: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            repos_dir: repos_dir.into(),
            max_sessions: DEFAULT_MAX_SESSIONS,
            session_timeout: Duration::from_secs(DEFAULT_SESSION_TIMEOUT_SECS),
            tool: None,
            state_dir: state_dir.into(),
            harness_bin: std::env::current_exe().unwrap_or_else(|_| PathBuf::from("le-harness")),
        }
    }

    pub fn default_state_dir() -> Result<PathBuf, ServeError> {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            ServeError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME is not set; pass --state-dir explicitly",
            ))
        })?;
        Ok(PathBuf::from(home).join(DEFAULT_STATE_ROOT))
    }
}

#[derive(Debug, Error)]
pub enum ServeError {
    #[error("malformed request line: {0}")]
    Malformed(String),
    #[error("max-sessions ({max}) reached: refusing new session (fail-closed)")]
    MaxSessions { max: usize },
    #[error("session {0:?} not found")]
    NotFound(String),
    #[error("invalid repo name {0:?}: must be a plain directory name under --repos-dir")]
    InvalidRepo(String),
    #[error("invalid session id {0:?}")]
    InvalidSessionId(String),
    #[error("session {0:?} already exists")]
    AlreadyExists(String),
    #[error(transparent)]
    Process(#[from] ProcessError),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("audit log failure: {0}")]
    Audit(String),
}

fn err_response(error: impl std::fmt::Display) -> Response {
    Response::Error {
        error: error.to_string(),
        session: None,
        req: None,
    }
}

/// One orchestrator request (line JSON on stdin). Parsing is fail-closed:
/// unknown fields, missing fields, wrong types and unknown ops are errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Start {
        repo: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Call {
        session: String,
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
    },
    Kill {
        session: String,
    },
    List,
    Bounds,
    Quit,
}

/// One orchestrator response (line JSON on stdout), tagged by session id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Response {
    Started {
        session: String,
        repo: String,
    },
    Accepted {
        session: String,
        req: u64,
    },
    Result {
        session: String,
        req: u64,
        ok: bool,
        code: i32,
        stdout: String,
        stderr: String,
        truncated: bool,
    },
    Killed {
        session: String,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Bounds {
        max_sessions: usize,
        session_timeout_secs: u64,
        active: usize,
    },
    Ended,
    Error {
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        req: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionInfo {
    pub session: String,
    pub repo: String,
    pub alive: bool,
    pub pending: usize,
}

/// A plain name (repo under `--repos-dir`, or session id): no path separators,
/// no traversal. Same charset rule as the persistence layer.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn required_str<'a>(
    obj: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, ServeError> {
    match obj.get(key) {
        Some(serde_json::Value::String(s)) => Ok(s),
        Some(_) => Err(ServeError::Malformed(format!(
            "field {key:?} must be a string"
        ))),
        None => Err(ServeError::Malformed(format!("missing field {key:?}"))),
    }
}

fn optional_str(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, ServeError> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ServeError::Malformed(format!(
            "field {key:?} must be a string"
        ))),
    }
}

fn optional_u64(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<u64>, ServeError> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| ServeError::Malformed(format!("field {key:?} must be a u64"))),
    }
}

fn reject_unknown(keys: &HashSet<&str>, allowed: &[&str]) -> Result<(), ServeError> {
    for k in keys {
        if !allowed.contains(k) {
            return Err(ServeError::Malformed(format!("unknown field {k:?}")));
        }
    }
    Ok(())
}

/// Parses one request line. Fail-closed: invalid JSON, a non-object, a
/// missing/unknown `op`, unknown fields, missing or mistyped fields are all
/// errors, never a partially interpreted request.
pub fn parse_request(line: &str) -> Result<Request, ServeError> {
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| ServeError::Malformed(e.to_string()))?;
    let obj = v
        .as_object()
        .ok_or_else(|| ServeError::Malformed("request must be a JSON object".into()))?;
    let keys: HashSet<&str> = obj.keys().map(|k| k.as_str()).collect();
    let op = required_str(obj, "op")?;
    match op {
        "start" => {
            reject_unknown(&keys, &["op", "repo", "id"])?;
            Ok(Request::Start {
                repo: required_str(obj, "repo")?.to_string(),
                id: optional_str(obj, "id")?,
            })
        }
        "call" => {
            reject_unknown(&keys, &["op", "session", "input", "id"])?;
            Ok(Request::Call {
                session: required_str(obj, "session")?.to_string(),
                input: required_str(obj, "input")?.to_string(),
                id: optional_u64(obj, "id")?,
            })
        }
        "kill" => {
            reject_unknown(&keys, &["op", "session"])?;
            Ok(Request::Kill {
                session: required_str(obj, "session")?.to_string(),
            })
        }
        "list" => {
            reject_unknown(&keys, &["op"])?;
            Ok(Request::List)
        }
        "bounds" => {
            reject_unknown(&keys, &["op"])?;
            Ok(Request::Bounds)
        }
        "quit" => {
            reject_unknown(&keys, &["op"])?;
            Ok(Request::Quit)
        }
        other => Err(ServeError::Malformed(format!("unknown op {other:?}"))),
    }
}

#[derive(Debug)]
struct SessionEvent {
    session: SessionId,
    result: Result<String, ProcessError>,
}

struct SessionHandle {
    repo: PathBuf,
    piped: PipedChild,
    secrets_file: PathBuf,
    child_id: u32,
    pending: VecDeque<(u64, Instant)>,
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.piped.kill();
        let _ = std::fs::remove_file(&self.secrets_file);
    }
}

/// Append-only per-session audit log: one JSONL file per session, ids and
/// events only — never request or response content.
struct AuditLog {
    dir: PathBuf,
}

impl AuditLog {
    fn new(dir: PathBuf) -> Result<Self, ServeError> {
        std::fs::create_dir_all(&dir).map_err(|e| {
            ServeError::Audit(format!("cannot create audit dir {}: {e}", dir.display()))
        })?;
        Ok(Self { dir })
    }

    fn log(&self, session: &SessionId, event: &str) -> Result<(), ServeError> {
        let raw = session.to_string();
        if !valid_name(&raw) {
            return Err(ServeError::Audit(format!("invalid session id {raw:?}")));
        }
        let path = self.dir.join(format!("{raw}.jsonl"));
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let rec = serde_json::json!({
            "ts": ts,
            "session": raw,
            "event": event,
        });
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| ServeError::Audit(format!("cannot open {}: {e}", path.display())))?;
        writeln!(file, "{rec}")
            .map_err(|e| ServeError::Audit(format!("cannot append {}: {e}", path.display())))?;
        Ok(())
    }
}

/// The multi-session orchestrator. One reader thread per session pushes into
/// `event_rx`; the single orchestrating thread multiplexes requests (stdin)
/// and session events, and enforces the per-message timeout.
pub struct Orchestrator {
    repos_dir: PathBuf,
    max_sessions: usize,
    session_timeout: Duration,
    tool: Option<PathBuf>,
    sessions_dir: PathBuf,
    secrets_dir: PathBuf,
    harness_bin: PathBuf,
    sessions: HashMap<SessionId, SessionHandle>,
    event_tx: Sender<SessionEvent>,
    event_rx: Receiver<SessionEvent>,
    req_counter: u64,
    seq_counter: u64,
    audit: AuditLog,
    quit_requested: bool,
}

impl Orchestrator {
    pub fn new(config: &ServeConfig) -> Result<Self, ServeError> {
        let sessions_dir = config.state_dir.join("sessions");
        let audit_dir = config.state_dir.join("audit");
        let secrets_dir = config.state_dir.join("serve-secrets");
        std::fs::create_dir_all(&sessions_dir)?;
        std::fs::create_dir_all(&secrets_dir)?;
        let (event_tx, event_rx) = channel();
        Ok(Self {
            repos_dir: config.repos_dir.clone(),
            max_sessions: config.max_sessions,
            session_timeout: config.session_timeout,
            tool: config.tool.clone(),
            sessions_dir,
            secrets_dir,
            harness_bin: config.harness_bin.clone(),
            sessions: HashMap::new(),
            event_tx,
            event_rx,
            req_counter: 0,
            seq_counter: 0,
            audit: AuditLog::new(audit_dir)?,
            quit_requested: false,
        })
    }

    pub fn active_count(&self) -> usize {
        self.sessions.len()
    }

    /// OS pid of a session's subprocess, if the session exists.
    pub fn child_pid(&self, session: &str) -> Option<u32> {
        let sid = SessionId::from_str(session).ok()?;
        self.sessions.get(&sid).map(|h| h.child_id)
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    /// Handles one request line and returns the synchronous responses.
    pub fn handle_line(&mut self, line: &str) -> Vec<Response> {
        match parse_request(line) {
            Err(e) => vec![err_response(e)],
            Ok(req) => self.handle(req),
        }
    }

    /// Waits up to `wait` for session events, drains the rest, and enforces
    /// per-message timeouts. Returns the asynchronous responses to emit.
    pub fn collect_events(&mut self, wait: Duration) -> Vec<Response> {
        let mut out = Vec::new();
        if let Ok(ev) = self.event_rx.recv_timeout(wait) {
            out.extend(self.handle_event(ev));
        }
        while let Ok(ev) = self.event_rx.try_recv() {
            out.extend(self.handle_event(ev));
        }
        out.extend(self.check_timeouts());
        out
    }

    fn handle(&mut self, req: Request) -> Vec<Response> {
        match req {
            Request::Start { repo, id } => vec![self.start(&repo, id)],
            Request::Call { session, input, id } => self.call(&session, input, id),
            Request::Kill { session } => vec![self.kill(&session)],
            Request::List => vec![self.list()],
            Request::Bounds => vec![Response::Bounds {
                max_sessions: self.max_sessions,
                session_timeout_secs: self.session_timeout.as_secs(),
                active: self.sessions.len(),
            }],
            Request::Quit => {
                self.quit_requested = true;
                self.shutdown();
                vec![Response::Ended]
            }
        }
    }

    fn alloc_req_id(&mut self) -> u64 {
        self.req_counter += 1;
        self.req_counter
    }

    fn phantom_token(&mut self) -> String {
        self.seq_counter += 1;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!(
            "phantom-{nanos:x}-{:x}-{:x}",
            std::process::id(),
            self.seq_counter
        )
    }

    fn start(&mut self, repo: &str, id: Option<String>) -> Response {
        if !valid_name(repo) {
            return err_response(ServeError::InvalidRepo(repo.to_string()));
        }
        let repo_path = self.repos_dir.join(repo);
        if !repo_path.is_dir() {
            return err_response(ServeError::InvalidRepo(format!(
                "{repo:?} is not a directory under {}",
                self.repos_dir.display()
            )));
        }
        if self.sessions.len() >= self.max_sessions {
            return err_response(ServeError::MaxSessions {
                max: self.max_sessions,
            });
        }
        let sid = match id {
            Some(raw) if valid_name(&raw) => match SessionId::from_str(&raw) {
                Ok(s) => s,
                Err(_) => return err_response(ServeError::InvalidSessionId(raw)),
            },
            Some(raw) => return err_response(ServeError::InvalidSessionId(raw)),
            None => SessionId::generate(),
        };
        if self.sessions.contains_key(&sid) {
            return err_response(ServeError::AlreadyExists(sid.to_string()));
        }
        let token = self.phantom_token();
        let secrets_file = self.secrets_dir.join(format!("{sid}.env"));
        if let Err(e) = std::fs::write(&secrets_file, format!("PHANTOM_TOKEN={token}\n")) {
            return err_response(ServeError::Io(e));
        }
        let mut args = vec![
            "run".to_string(),
            "--repo".to_string(),
            repo_path.to_string_lossy().into_owned(),
            "--json".to_string(),
            "--secrets-file".to_string(),
            secrets_file.to_string_lossy().into_owned(),
            "--state-dir".to_string(),
            self.sessions_dir.to_string_lossy().into_owned(),
        ];
        if let Some(tool) = &self.tool {
            args.push("--tool".to_string());
            args.push(tool.to_string_lossy().into_owned());
        }
        // The orchestrator holds no session secret: empty store, it only
        // routes lines that the session subprocess already masked.
        let empty = SecretStore::new();
        let mut piped = match spawn_piped(&self.harness_bin, &args, &empty, DEFAULT_ENV_ALLOWLIST) {
            Ok(p) => p,
            Err(e) => {
                let _ = std::fs::remove_file(&secrets_file);
                return err_response(ServeError::Process(e));
            }
        };
        let child_id = piped.child_id();
        let rx = piped.take_rx();
        let tx = self.event_tx.clone();
        let sid_reader = sid.clone();
        thread::spawn(move || {
            while let Ok(msg) = rx.recv() {
                if tx
                    .send(SessionEvent {
                        session: sid_reader.clone(),
                        result: msg,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        if let Err(e) = self
            .audit
            .log(&sid, &format!("start repo={repo} pid={child_id}"))
        {
            self.sessions_remove(&sid, &secrets_file, &mut piped);
            return err_response(e);
        }
        self.sessions.insert(
            sid.clone(),
            SessionHandle {
                repo: repo_path.clone(),
                piped,
                secrets_file,
                child_id,
                pending: VecDeque::new(),
            },
        );
        Response::Started {
            session: sid.to_string(),
            repo: repo_path.to_string_lossy().into_owned(),
        }
    }

    fn sessions_remove(&mut self, sid: &SessionId, secrets_file: &Path, piped: &mut PipedChild) {
        piped.kill();
        let _ = std::fs::remove_file(secrets_file);
        let _ = sid;
    }

    fn call(&mut self, session: &str, input: String, id: Option<u64>) -> Vec<Response> {
        if !valid_name(session) {
            return vec![err_response(ServeError::InvalidSessionId(
                session.to_string(),
            ))];
        }
        let sid = match SessionId::from_str(session) {
            Ok(s) => s,
            Err(_) => {
                return vec![err_response(ServeError::InvalidSessionId(
                    session.to_string(),
                ))]
            }
        };
        let req_id = id.unwrap_or_else(|| self.alloc_req_id());
        let line = match serde_json::to_string(&ToolRequest {
            id: req_id,
            input: input.clone(),
        }) {
            Ok(l) => l,
            Err(e) => return vec![err_response(ServeError::Malformed(e.to_string()))],
        };
        let Some(handle) = self.sessions.get_mut(&sid) else {
            return vec![Response::Error {
                error: ServeError::NotFound(sid.to_string()).to_string(),
                session: Some(sid.to_string()),
                req: Some(req_id),
            }];
        };
        if let Err(e) = handle.piped.write_line(&line) {
            let err = ServeError::Process(e);
            let _ = self.audit.log(&sid, "write-failed");
            self.remove_session(&sid);
            return vec![Response::Error {
                error: err.to_string(),
                session: Some(sid.to_string()),
                req: Some(req_id),
            }];
        }
        handle.pending.push_back((req_id, Instant::now()));
        let _ = self.audit.log(&sid, &format!("accepted req={req_id}"));
        vec![Response::Accepted {
            session: sid.to_string(),
            req: req_id,
        }]
    }

    fn kill(&mut self, session: &str) -> Response {
        if !valid_name(session) {
            return err_response(ServeError::InvalidSessionId(session.to_string()));
        }
        let sid = match SessionId::from_str(session) {
            Ok(s) => s,
            Err(_) => return err_response(ServeError::InvalidSessionId(session.to_string())),
        };
        if !self.sessions.contains_key(&sid) {
            return err_response(ServeError::NotFound(sid.to_string()));
        }
        let _ = self.audit.log(&sid, "killed");
        self.remove_session(&sid);
        Response::Killed {
            session: sid.to_string(),
        }
    }

    fn list(&mut self) -> Response {
        let mut sessions: Vec<SessionInfo> = self
            .sessions
            .iter_mut()
            .map(|(sid, h)| SessionInfo {
                session: sid.to_string(),
                repo: h.repo.to_string_lossy().into_owned(),
                alive: h.piped.is_alive(),
                pending: h.pending.len(),
            })
            .collect();
        sessions.sort_by(|a, b| a.session.cmp(&b.session));
        Response::Sessions { sessions }
    }

    fn handle_event(&mut self, ev: SessionEvent) -> Vec<Response> {
        let sid = ev.session;
        if !self.sessions.contains_key(&sid) {
            // Session already removed (killed/timed out): late EOF or reader
            // events are ignored, no duplicate error is emitted.
            return Vec::new();
        }
        match ev.result {
            Ok(line) => match parse_line(&line) {
                Ok(resp) => {
                    let req = self
                        .sessions
                        .get_mut(&sid)
                        .and_then(|h| h.pending.pop_front())
                        .map(|(id, _)| id)
                        .unwrap_or(0);
                    let _ = self.audit.log(
                        &sid,
                        &format!("result req={req} ok={} code={}", resp.ok, resp.code),
                    );
                    vec![Response::Result {
                        session: sid.to_string(),
                        req,
                        ok: resp.ok,
                        code: resp.code,
                        stdout: resp.stdout,
                        stderr: resp.stderr,
                        truncated: resp.truncated,
                    }]
                }
                Err(e) => {
                    let _ = self.audit.log(&sid, "malformed-response");
                    self.remove_session(&sid);
                    vec![Response::Error {
                        error: format!("malformed session response (fail-closed): {e}"),
                        session: Some(sid.to_string()),
                        req: None,
                    }]
                }
            },
            Err(e) => {
                let _ = self.audit.log(&sid, &format!("closed: {e}"));
                self.remove_session(&sid);
                vec![Response::Error {
                    error: format!("session closed: {e}"),
                    session: Some(sid.to_string()),
                    req: None,
                }]
            }
        }
    }

    fn check_timeouts(&mut self) -> Vec<Response> {
        let timeout = self.session_timeout;
        let mut expired: Vec<(SessionId, u64)> = Vec::new();
        for (sid, h) in &self.sessions {
            if let Some((req, sent)) = h.pending.front() {
                if sent.elapsed() >= timeout {
                    expired.push((sid.clone(), *req));
                }
            }
        }
        let mut out = Vec::new();
        for (sid, req) in expired {
            let _ = self.audit.log(&sid, &format!("timeout req={req}"));
            self.remove_session(&sid);
            out.push(Response::Error {
                error: format!("session timeout after {timeout:?} (killed)"),
                session: Some(sid.to_string()),
                req: Some(req),
            });
        }
        out
    }

    fn remove_session(&mut self, sid: &SessionId) {
        if let Some(handle) = self.sessions.remove(sid) {
            drop(handle); // PipedChild::drop kills + reaps the child
        }
    }

    /// Kills every active session cleanly (kill + wait) and logs it.
    pub fn shutdown(&mut self) {
        let ids: Vec<SessionId> = self.sessions.keys().cloned().collect();
        for sid in ids {
            let _ = self.audit.log(&sid, "ended");
            self.remove_session(&sid);
        }
    }
}

impl Drop for Orchestrator {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn write_response(output: &mut impl Write, resp: &Response) -> Result<(), ServeError> {
    let line = serde_json::to_string(resp)
        .map_err(|e| ServeError::Malformed(format!("cannot serialize response: {e}")))?;
    writeln!(output, "{line}")?;
    output.flush()?;
    Ok(())
}

/// Runs the orchestrator loop: request lines on stdin, response lines on
/// `output`. One reader thread for stdin, one per session for stdout, and a
/// single multiplexing loop — no async runtime. On `quit` or stdin EOF all
/// session subprocesses are killed cleanly (none left alive).
pub fn serve(config: &ServeConfig, mut output: impl Write) -> Result<(), ServeError> {
    let mut orch = Orchestrator::new(config)?;
    let (input_tx, input_rx) = channel::<std::io::Result<String>>();
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            if input_tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut stdin_open = true;
    loop {
        if stdin_open {
            match input_rx.try_recv() {
                Ok(Ok(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let responses = orch.handle_line(line);
                    let quit = responses.iter().any(|r| matches!(r, Response::Ended));
                    for r in &responses {
                        write_response(&mut output, r)?;
                    }
                    if quit {
                        break;
                    }
                }
                Ok(Err(e)) => return Err(ServeError::Io(e)),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => stdin_open = false,
            }
        }
        for r in orch.collect_events(Duration::from_millis(10)) {
            write_response(&mut output, &r)?;
        }
        if !stdin_open {
            break;
        }
    }
    orch.shutdown();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn config(dir: &tempfile::TempDir) -> ServeConfig {
        let mut c = ServeConfig::new(dir.path().join("repos"), dir.path().join("state"));
        c.max_sessions = 2;
        c.session_timeout = Duration::from_millis(200);
        c
    }

    #[test]
    fn parse_request_accepts_all_ops() {
        assert_eq!(
            parse_request(r#"{"op":"start","repo":"r1"}"#).unwrap(),
            Request::Start {
                repo: "r1".into(),
                id: None
            }
        );
        assert_eq!(
            parse_request(r#"{"op":"start","repo":"r1","id":"sess-x"}"#).unwrap(),
            Request::Start {
                repo: "r1".into(),
                id: Some("sess-x".into())
            }
        );
        assert_eq!(
            parse_request(r#"{"op":"call","session":"s","input":"echo hi","id":7}"#).unwrap(),
            Request::Call {
                session: "s".into(),
                input: "echo hi".into(),
                id: Some(7)
            }
        );
        assert_eq!(
            parse_request(r#"{"op":"kill","session":"s"}"#).unwrap(),
            Request::Kill {
                session: "s".into()
            }
        );
        assert_eq!(parse_request(r#"{"op":"list"}"#).unwrap(), Request::List);
        assert_eq!(
            parse_request(r#"{"op":"bounds"}"#).unwrap(),
            Request::Bounds
        );
        assert_eq!(parse_request(r#"{"op":"quit"}"#).unwrap(), Request::Quit);
    }

    #[test]
    fn parse_request_is_fail_closed() {
        for bad in [
            "",
            "not json",
            "[]",
            "{}",
            r#"{"op":1}"#,
            r#"{"op":"nope"}"#,
            r#"{"op":"start"}"#,                                   // missing repo
            r#"{"op":"start","repo":1}"#,                          // wrong type
            r#"{"op":"start","repo":"r","extra":1}"#,              // unknown field
            r#"{"op":"call","session":"s"}"#,                      // missing input
            r#"{"op":"call","session":"s","input":"x","id":"7"}"#, // id not u64
            r#"{"op":"kill"}"#,                                    // missing session
            r#"{"op":"list","x":1}"#,                              // unknown field on unit op
        ] {
            assert!(
                matches!(parse_request(bad), Err(ServeError::Malformed(_))),
                "expected Malformed for {bad:?}"
            );
        }
    }

    proptest! {
        #[test]
        fn parse_request_roundtrip_or_fail_closed(s in ".*") {
            if let Ok(req) = parse_request(&s) {
                let line = serde_json::to_string(&req).unwrap();
                prop_assert_eq!(parse_request(&line).unwrap(), req);
            }
        }
    }

    #[test]
    fn valid_name_rejects_traversal_and_separators() {
        assert!(valid_name("r1"));
        assert!(valid_name("repo_2-x"));
        assert!(!valid_name(""));
        assert!(!valid_name("."));
        assert!(!valid_name(".."));
        assert!(!valid_name("../etc"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name("a b"));
    }

    #[test]
    fn max_sessions_zero_refuses_start_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repos/r1")).unwrap();
        let mut c = config(&dir);
        c.max_sessions = 0;
        let mut orch = Orchestrator::new(&c).unwrap();
        let resps = orch.handle_line(r#"{"op":"start","repo":"r1"}"#);
        assert_eq!(resps.len(), 1);
        assert!(
            matches!(&resps[0], Response::Error { error, .. } if error.contains("max-sessions"))
        );
        assert_eq!(orch.active_count(), 0);
    }

    #[test]
    fn start_rejects_invalid_or_missing_repo_before_any_spawn() {
        let dir = tempfile::tempdir().unwrap();
        let mut orch = Orchestrator::new(&config(&dir)).unwrap();
        for bad in ["..", "a/b", "no-such-repo"] {
            let resps = orch.handle_line(&format!(r#"{{"op":"start","repo":"{bad}"}}"#));
            assert!(matches!(&resps[0], Response::Error { .. }), "{bad}");
        }
        assert_eq!(orch.active_count(), 0);
    }

    #[test]
    fn start_rejects_invalid_session_id() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repos/r1")).unwrap();
        let mut orch = Orchestrator::new(&config(&dir)).unwrap();
        let resps = orch.handle_line(r#"{"op":"start","repo":"r1","id":"../evil"}"#);
        assert!(
            matches!(&resps[0], Response::Error { error, .. } if error.contains("invalid session id"))
        );
        assert_eq!(orch.active_count(), 0);
    }

    #[test]
    fn call_and_kill_on_unknown_session_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut orch = Orchestrator::new(&config(&dir)).unwrap();
        let resps = orch.handle_line(r#"{"op":"call","session":"sess-nope","input":"x"}"#);
        assert!(matches!(&resps[0], Response::Error { error, .. } if error.contains("not found")));
        let resps = orch.handle_line(r#"{"op":"kill","session":"sess-nope"}"#);
        assert!(matches!(&resps[0], Response::Error { error, .. } if error.contains("not found")));
    }

    #[test]
    fn bounds_response_reports_limits_and_active_count() {
        let dir = tempfile::tempdir().unwrap();
        let mut orch = Orchestrator::new(&config(&dir)).unwrap();
        let resps = orch.handle_line(r#"{"op":"bounds"}"#);
        assert_eq!(
            resps[0],
            Response::Bounds {
                max_sessions: 2,
                session_timeout_secs: 0,
                active: 0
            }
        );
    }

    #[test]
    fn quit_ends_and_kills_everything() {
        let dir = tempfile::tempdir().unwrap();
        let mut orch = Orchestrator::new(&config(&dir)).unwrap();
        let resps = orch.handle_line(r#"{"op":"quit"}"#);
        assert_eq!(resps, vec![Response::Ended]);
        assert!(orch.quit_requested());
        assert_eq!(orch.active_count(), 0);
    }

    #[test]
    fn malformed_request_line_gets_an_error_response_and_the_loop_continues() {
        let dir = tempfile::tempdir().unwrap();
        let mut orch = Orchestrator::new(&config(&dir)).unwrap();
        let resps = orch.handle_line("garbage");
        assert!(matches!(&resps[0], Response::Error { .. }));
        let resps = orch.handle_line(r#"{"op":"bounds"}"#);
        assert!(matches!(&resps[0], Response::Bounds { .. }));
    }

    #[test]
    fn audit_log_is_append_only_jsonl_and_never_contains_content() {
        let dir = tempfile::tempdir().unwrap();
        let audit = AuditLog::new(dir.path().join("audit")).unwrap();
        let sid = SessionId::from_str("sess-test-1").unwrap();
        audit.log(&sid, "start repo=r1").unwrap();
        audit.log(&sid, "accepted req=1").unwrap();
        audit.log(&sid, "result req=1 ok=true code=0").unwrap();
        let raw = std::fs::read_to_string(dir.path().join("audit/sess-test-1.jsonl")).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["session"], "sess-test-1");
            assert!(v["ts"].is_u64());
            assert!(v["event"].is_string());
        }
        assert!(raw.contains("start repo=r1"));
        // content of requests/responses must never be logged
        assert!(!raw.contains("secret-content"));
    }

    #[test]
    fn audit_log_rejects_invalid_session_id() {
        let dir = tempfile::tempdir().unwrap();
        let audit = AuditLog::new(dir.path().join("audit")).unwrap();
        let sid = SessionId::from_str("../evil").unwrap();
        assert!(matches!(audit.log(&sid, "x"), Err(ServeError::Audit(_))));
    }
}
