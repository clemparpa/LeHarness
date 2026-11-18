//! File-based persistence for session snapshots (step 5): one JSON file per
//! session under a state directory, written atomically (tmp + sync + rename),
//! so a checkpoint survives a process crash. Fail-closed: a missing file is
//! `Ok(None)`, a corrupt file or an invalid session id is an error, never
//! silently ignored.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::session::{Persistence, PersistenceError, SessionId, Snapshot};

pub const DEFAULT_STATE_DIR: &str = ".local/share/le-harness/sessions";

pub struct FilePersistence {
    dir: PathBuf,
}

impl FilePersistence {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, PersistenceError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|e| {
            PersistenceError::Backend(format!("cannot create state dir {}: {e}", dir.display()))
        })?;
        Ok(Self { dir })
    }

    pub fn default_dir() -> Result<PathBuf, PersistenceError> {
        let home = std::env::var_os("HOME").ok_or_else(|| {
            PersistenceError::Backend("HOME is not set; pass --state-dir explicitly".to_string())
        })?;
        Ok(PathBuf::from(home).join(DEFAULT_STATE_DIR))
    }

    fn file_for(&self, id: &SessionId) -> Result<PathBuf, PersistenceError> {
        let raw = id.to_string();
        if raw.is_empty()
            || raw == "."
            || raw == ".."
            || !raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(PersistenceError::Backend(format!(
                "invalid session id {raw:?}"
            )));
        }
        Ok(self.dir.join(format!("{raw}.json")))
    }
}

impl Persistence for FilePersistence {
    fn save(&self, snapshot: &Snapshot) -> Result<(), PersistenceError> {
        let path = self.file_for(&snapshot.id)?;
        let tmp = path.with_extension("json.tmp");
        let data = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| PersistenceError::Backend(format!("cannot serialize snapshot: {e}")))?;
        {
            let mut file = fs::File::create(&tmp).map_err(|e| {
                PersistenceError::Backend(format!("cannot write {}: {e}", tmp.display()))
            })?;
            file.write_all(&data).map_err(|e| {
                PersistenceError::Backend(format!("cannot write {}: {e}", tmp.display()))
            })?;
            file.sync_all().map_err(|e| {
                PersistenceError::Backend(format!("cannot sync {}: {e}", tmp.display()))
            })?;
        }
        fs::rename(&tmp, &path).map_err(|e| {
            PersistenceError::Backend(format!(
                "cannot rename {} to {}: {e}",
                tmp.display(),
                path.display()
            ))
        })?;
        Ok(())
    }

    fn load(&self, id: &SessionId) -> Result<Option<Snapshot>, PersistenceError> {
        let path = self.file_for(id)?;
        match fs::read(&path) {
            Ok(data) => {
                let snapshot = serde_json::from_slice(&data).map_err(|e| {
                    PersistenceError::Backend(format!("corrupt snapshot {}: {e}", path.display()))
                })?;
                Ok(Some(snapshot))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(PersistenceError::Backend(format!(
                "cannot read {}: {e}",
                path.display()
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionState;
    use std::str::FromStr;

    fn snapshot(id: &str, checkpoints: u64) -> Snapshot {
        Snapshot {
            id: SessionId::from_str(id).unwrap(),
            root: PathBuf::from("/tmp/repo"),
            state: SessionState::Running,
            checkpoints,
        }
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path().join("state")).unwrap();
        let snap = snapshot("sess-a-0", 2);
        store.save(&snap).unwrap();
        let loaded = store.load(&snap.id).unwrap();
        assert_eq!(loaded, Some(snap));
    }

    #[test]
    fn load_unknown_id_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path()).unwrap();
        let id = SessionId::from_str("sess-nope-0").unwrap();
        assert_eq!(store.load(&id).unwrap(), None);
    }

    #[test]
    fn save_is_an_upsert_last_snapshot_wins() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path()).unwrap();
        let id = SessionId::from_str("sess-b-1").unwrap();
        store.save(&snapshot("sess-b-1", 1)).unwrap();
        store.save(&snapshot("sess-b-1", 7)).unwrap();
        let loaded = store.load(&id).unwrap().unwrap();
        assert_eq!(loaded.checkpoints, 7);
    }

    #[test]
    fn corrupt_file_is_an_error_never_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path()).unwrap();
        let id = SessionId::from_str("sess-c-2").unwrap();
        fs::write(dir.path().join("sess-c-2.json"), b"{not json").unwrap();
        assert!(matches!(store.load(&id), Err(PersistenceError::Backend(_))));
    }

    #[test]
    fn invalid_ids_are_rejected_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path()).unwrap();
        for bad in ["../evil", "a/b", ".", "..", "sess x", "sess\nx"] {
            let id = SessionId::from_str(bad).unwrap();
            assert!(
                store.save(&snapshot(bad, 0)).is_err(),
                "save must reject {bad:?}"
            );
            assert!(store.load(&id).is_err(), "load must reject {bad:?}");
        }
    }

    #[test]
    fn new_creates_a_nested_directory() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("deep").join("state");
        FilePersistence::new(&nested).unwrap();
        assert!(nested.is_dir());
    }

    #[test]
    fn save_leaves_no_tmp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path()).unwrap();
        store.save(&snapshot("sess-d-3", 1)).unwrap();
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["sess-d-3.json".to_string()]);
    }

    #[test]
    fn saved_file_is_json_with_expected_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = FilePersistence::new(dir.path()).unwrap();
        store.save(&snapshot("sess-e-4", 5)).unwrap();
        let raw = fs::read_to_string(dir.path().join("sess-e-4.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["id"], "sess-e-4");
        assert_eq!(v["root"], "/tmp/repo");
        assert_eq!(v["state"], "Running");
        assert_eq!(v["checkpoints"], 5);
    }

    #[test]
    fn default_dir_is_under_home() {
        let home = std::env::var("HOME").expect("HOME set in test env");
        let dir = FilePersistence::default_dir().unwrap();
        assert_eq!(
            dir,
            PathBuf::from(home).join(".local/share/le-harness/sessions")
        );
    }
}
