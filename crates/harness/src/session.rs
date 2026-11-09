//! Session core: explicit lifecycle (start / checkpoint / end / resume) emitted
//! through hooks, with persistence injected by the calling layer. The core never
//! touches the disk itself.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn generate() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(format!("sess-{nanos:x}-{n:x}"))
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Running,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub id: SessionId,
    pub root: PathBuf,
    pub state: SessionState,
    pub checkpoints: u64,
}

pub trait Hooks {
    fn on_start(&mut self, _snapshot: &Snapshot) {}
    fn on_checkpoint(&mut self, _snapshot: &Snapshot) {}
    fn on_end(&mut self, _snapshot: &Snapshot) {}
    fn on_resume(&mut self, _snapshot: &Snapshot) {}
}

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("persistence backend failure: {0}")]
    Backend(String),
}

pub trait Persistence {
    fn save(&mut self, snapshot: &Snapshot) -> Result<(), PersistenceError>;
    fn load(&mut self, id: &SessionId) -> Result<Option<Snapshot>, PersistenceError>;
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("persistence failure: {0}")]
    Persistence(#[from] PersistenceError),
    #[error("session {0} not found")]
    NotFound(SessionId),
    #[error("session {0} already ended")]
    AlreadyEnded(SessionId),
    #[error("cannot {action} a session in state {state:?}")]
    InvalidState {
        action: &'static str,
        state: SessionState,
    },
    #[error("root mismatch for session {id}: expected {expected}, got {got}")]
    RootMismatch {
        id: SessionId,
        expected: PathBuf,
        got: PathBuf,
    },
}

pub struct Session {
    snapshot: Snapshot,
    hooks: Box<dyn Hooks>,
    store: Box<dyn Persistence>,
}

impl Session {
    pub fn start(
        id: SessionId,
        root: PathBuf,
        hooks: impl Hooks + 'static,
        store: impl Persistence + 'static,
    ) -> Result<Self, SessionError> {
        let snapshot = Snapshot {
            id,
            root,
            state: SessionState::Running,
            checkpoints: 0,
        };
        let mut session = Self {
            snapshot,
            hooks: Box::new(hooks),
            store: Box::new(store),
        };
        session.store.save(&session.snapshot)?;
        session.hooks.on_start(&session.snapshot);
        Ok(session)
    }

    pub fn resume(
        id: SessionId,
        root: PathBuf,
        hooks: impl Hooks + 'static,
        store: impl Persistence + 'static,
    ) -> Result<Self, SessionError> {
        let mut store = Box::new(store);
        let mut hooks = Box::new(hooks);
        let mut snapshot = store
            .load(&id)?
            .ok_or_else(|| SessionError::NotFound(id.clone()))?;
        if snapshot.state == SessionState::Ended {
            return Err(SessionError::AlreadyEnded(id));
        }
        if snapshot.root != root {
            return Err(SessionError::RootMismatch {
                id,
                expected: snapshot.root.clone(),
                got: root,
            });
        }
        snapshot.state = SessionState::Running;
        store.save(&snapshot)?;
        hooks.on_resume(&snapshot);
        Ok(Self {
            snapshot,
            hooks,
            store,
        })
    }

    pub fn checkpoint(&mut self) -> Result<u64, SessionError> {
        if self.snapshot.state == SessionState::Ended {
            return Err(SessionError::InvalidState {
                action: "checkpoint",
                state: self.snapshot.state,
            });
        }
        self.snapshot.checkpoints += 1;
        self.store.save(&self.snapshot)?;
        self.hooks.on_checkpoint(&self.snapshot);
        Ok(self.snapshot.checkpoints)
    }

    pub fn end(&mut self) -> Result<(), SessionError> {
        if self.snapshot.state == SessionState::Ended {
            return Err(SessionError::AlreadyEnded(self.snapshot.id.clone()));
        }
        self.snapshot.state = SessionState::Ended;
        self.store.save(&self.snapshot)?;
        self.hooks.on_end(&self.snapshot);
        Ok(())
    }

    pub fn id(&self) -> &SessionId {
        &self.snapshot.id
    }

    pub fn root(&self) -> &Path {
        &self.snapshot.root
    }

    pub fn state(&self) -> SessionState {
        self.snapshot.state
    }

    pub fn checkpoints(&self) -> u64 {
        self.snapshot.checkpoints
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type SharedLog<T> = Rc<RefCell<Vec<T>>>;

    struct MemoryStore {
        saved: SharedLog<Snapshot>,
        fail_on_save: bool,
        fail_on_load: bool,
    }

    impl MemoryStore {
        fn new(saved: SharedLog<Snapshot>) -> Self {
            Self {
                saved,
                fail_on_save: false,
                fail_on_load: false,
            }
        }
    }

    impl Persistence for MemoryStore {
        fn save(&mut self, snapshot: &Snapshot) -> Result<(), PersistenceError> {
            if self.fail_on_save {
                return Err(PersistenceError::Backend("save failed".into()));
            }
            self.saved.borrow_mut().push(snapshot.clone());
            Ok(())
        }

        fn load(&mut self, id: &SessionId) -> Result<Option<Snapshot>, PersistenceError> {
            if self.fail_on_load {
                return Err(PersistenceError::Backend("load failed".into()));
            }
            Ok(self
                .saved
                .borrow()
                .iter()
                .rev()
                .find(|s| s.id == *id)
                .cloned())
        }
    }

    struct RecordingHooks {
        events: SharedLog<String>,
    }

    impl RecordingHooks {
        fn new(events: SharedLog<String>) -> Self {
            Self { events }
        }
    }

    impl Hooks for RecordingHooks {
        fn on_start(&mut self, s: &Snapshot) {
            self.events.borrow_mut().push(format!("start:{}", s.id));
        }
        fn on_checkpoint(&mut self, s: &Snapshot) {
            self.events
                .borrow_mut()
                .push(format!("checkpoint:{}", s.checkpoints));
        }
        fn on_end(&mut self, s: &Snapshot) {
            self.events.borrow_mut().push(format!("end:{}", s.id));
        }
        fn on_resume(&mut self, s: &Snapshot) {
            self.events
                .borrow_mut()
                .push(format!("resume:{}:{}", s.id, s.checkpoints));
        }
    }

    fn root() -> PathBuf {
        PathBuf::from("/tmp/repo")
    }

    fn fixtures() -> (SharedLog<Snapshot>, SharedLog<String>) {
        (
            Rc::new(RefCell::new(Vec::new())),
            Rc::new(RefCell::new(Vec::new())),
        )
    }

    #[test]
    fn start_emits_on_start_and_persists_initial_snapshot() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let session = Session::start(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        assert_eq!(session.id(), &id);
        assert_eq!(session.root(), root().as_path());
        assert_eq!(session.state(), SessionState::Running);
        assert_eq!(session.checkpoints(), 0);
        assert_eq!(*events.borrow(), vec![format!("start:{id}")]);
        let saved = saved.borrow();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].state, SessionState::Running);
        assert_eq!(saved[0].checkpoints, 0);
    }

    #[test]
    fn start_failure_on_save_blocks_session_and_emits_no_hook() {
        let (saved, events) = fixtures();
        let mut store = MemoryStore::new(saved);
        store.fail_on_save = true;
        let result = Session::start(
            SessionId::generate(),
            root(),
            RecordingHooks::new(events.clone()),
            store,
        );
        assert!(matches!(result, Err(SessionError::Persistence(_))));
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn checkpoint_increments_persists_and_emits_hook_in_order() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let mut session = Session::start(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        assert_eq!(session.checkpoint().unwrap(), 1);
        assert_eq!(session.checkpoint().unwrap(), 2);
        assert_eq!(session.checkpoints(), 2);
        assert_eq!(
            *events.borrow(),
            vec![
                format!("start:{id}"),
                "checkpoint:1".to_string(),
                "checkpoint:2".to_string()
            ]
        );
        assert_eq!(saved.borrow().len(), 3);
        assert_eq!(saved.borrow().last().unwrap().checkpoints, 2);
    }

    #[test]
    fn end_marks_ended_persists_and_double_end_is_rejected() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let mut session = Session::start(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        session.end().unwrap();
        assert_eq!(session.state(), SessionState::Ended);
        assert!(matches!(session.end(), Err(SessionError::AlreadyEnded(ref eid)) if *eid == id));
        assert_eq!(
            *events.borrow(),
            vec![format!("start:{id}"), format!("end:{id}")]
        );
        assert_eq!(saved.borrow().last().unwrap().state, SessionState::Ended);
    }

    #[test]
    fn checkpoint_after_end_is_rejected() {
        let (saved, events) = fixtures();
        let mut session = Session::start(
            SessionId::generate(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        session.end().unwrap();
        assert!(matches!(
            session.checkpoint(),
            Err(SessionError::InvalidState {
                action: "checkpoint",
                state: SessionState::Ended
            })
        ));
    }

    #[test]
    fn resume_running_session_restores_state_and_emits_hook() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let mut session = Session::start(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        session.checkpoint().unwrap();
        session.checkpoint().unwrap();
        session.checkpoint().unwrap();
        drop(session);

        let resume_events: SharedLog<String> = Rc::new(RefCell::new(Vec::new()));
        let resumed = Session::resume(
            id.clone(),
            root(),
            RecordingHooks::new(resume_events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        assert_eq!(resumed.state(), SessionState::Running);
        assert_eq!(resumed.checkpoints(), 3);
        assert_eq!(resumed.id(), &id);
        assert_eq!(resumed.root(), root().as_path());
        assert_eq!(*resume_events.borrow(), vec![format!("resume:{id}:3")]);
    }

    #[test]
    fn resume_unknown_id_is_not_found() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let result = Session::resume(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved),
        );
        assert!(matches!(result, Err(SessionError::NotFound(ref eid)) if *eid == id));
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn resume_ended_session_is_rejected() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let mut session = Session::start(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        session.end().unwrap();
        drop(session);

        let result = Session::resume(
            id.clone(),
            root(),
            RecordingHooks::new(Rc::new(RefCell::new(Vec::new()))),
            MemoryStore::new(saved),
        );
        assert!(matches!(result, Err(SessionError::AlreadyEnded(ref eid)) if *eid == id));
    }

    #[test]
    fn resume_with_different_root_is_rejected() {
        let (saved, events) = fixtures();
        let id = SessionId::generate();
        let session = Session::start(
            id.clone(),
            root(),
            RecordingHooks::new(events.clone()),
            MemoryStore::new(saved.clone()),
        )
        .unwrap();
        drop(session);

        let result = Session::resume(
            id,
            PathBuf::from("/tmp/other"),
            RecordingHooks::new(Rc::new(RefCell::new(Vec::new()))),
            MemoryStore::new(saved),
        );
        assert!(matches!(result, Err(SessionError::RootMismatch { .. })));
    }

    #[test]
    fn load_failure_is_fail_closed() {
        let (saved, events) = fixtures();
        let mut store = MemoryStore::new(saved);
        store.fail_on_load = true;
        let result = Session::resume(
            SessionId::generate(),
            root(),
            RecordingHooks::new(events.clone()),
            store,
        );
        assert!(matches!(result, Err(SessionError::Persistence(_))));
        assert!(events.borrow().is_empty());
    }

    #[test]
    fn generated_ids_are_unique() {
        let a = SessionId::generate();
        let b = SessionId::generate();
        assert_ne!(a, b);
    }
}
