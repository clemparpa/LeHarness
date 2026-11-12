//! Repository policy: `trusted.txt` and `sensitives.txt` live in the agent's
//! repo, are read by the harness at session start, and are handed to the tools
//! (the harness declares the policy, the tools enforce it). Fail-closed: a
//! missing or unreadable policy file aborts the session instead of silently
//! running with an empty policy.

use std::path::{Path, PathBuf};

use thiserror::Error;

pub const TRUSTED_FILE: &str = "trusted.txt";
pub const SENSITIVES_FILE: &str = "sensitives.txt";

#[derive(Debug, Error)]
pub enum PolicyError {
    #[error("policy file {path} is missing (fail-closed: no declared policy, no session)")]
    Missing { path: PathBuf },
    #[error("cannot read policy file {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("repo root {0} is not a directory")]
    NotADirectory(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoPolicy {
    pub trusted: Vec<String>,
    pub sensitives: Vec<String>,
}

impl RepoPolicy {
    /// Loads `trusted.txt` and `sensitives.txt` from `root`. One entry per
    /// line; blank lines and `#` comments are ignored. Fail-closed: the root
    /// must be a directory and both files must exist and be readable.
    pub fn load(root: impl AsRef<Path>) -> Result<Self, PolicyError> {
        let root = root.as_ref();
        if !root.is_dir() {
            return Err(PolicyError::NotADirectory(root.to_path_buf()));
        }
        let trusted = read_entries(&root.join(TRUSTED_FILE))?;
        let sensitives = read_entries(&root.join(SENSITIVES_FILE))?;
        Ok(Self {
            trusted,
            sensitives,
        })
    }

    pub fn trusted_file(&self, root: impl AsRef<Path>) -> PathBuf {
        root.as_ref().join(TRUSTED_FILE)
    }

    pub fn sensitives_file(&self, root: impl AsRef<Path>) -> PathBuf {
        root.as_ref().join(SENSITIVES_FILE)
    }
}

fn read_entries(path: &Path) -> Result<Vec<String>, PolicyError> {
    if !path.is_file() {
        return Err(PolicyError::Missing {
            path: path.to_path_buf(),
        });
    }
    let content = std::fs::read_to_string(path).map_err(|source| PolicyError::ReadFailed {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_repo(dir: &tempfile::TempDir, trusted: &str, sensitives: &str) -> PathBuf {
        let root = dir.path().to_path_buf();
        std::fs::write(root.join(TRUSTED_FILE), trusted).unwrap();
        std::fs::write(root.join(SENSITIVES_FILE), sensitives).unwrap();
        root
    }

    #[test]
    fn load_reads_entries_and_skips_blanks_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let root = write_repo(
            &dir,
            "# trusted commands\nrun-tests\n\n  build  \n",
            "# sensitive files\n.*\\.pem\ncredentials.json\n",
        );
        let policy = RepoPolicy::load(&root).unwrap();
        assert_eq!(policy.trusted, vec!["run-tests", "build"]);
        assert_eq!(policy.sensitives, vec![r".*\.pem", "credentials.json"]);
    }

    #[test]
    fn missing_trusted_file_is_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join(SENSITIVES_FILE), "x\n").unwrap();
        let err = RepoPolicy::load(&root).unwrap_err();
        assert!(matches!(err, PolicyError::Missing { .. }));
    }

    #[test]
    fn missing_sensitives_file_is_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join(TRUSTED_FILE), "x\n").unwrap();
        let err = RepoPolicy::load(&root).unwrap_err();
        assert!(matches!(err, PolicyError::Missing { .. }));
    }

    #[test]
    fn missing_repo_root_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("does-not-exist");
        let err = RepoPolicy::load(&root).unwrap_err();
        assert!(matches!(err, PolicyError::NotADirectory(_)));
    }

    #[test]
    fn file_as_root_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "x").unwrap();
        let err = RepoPolicy::load(&file).unwrap_err();
        assert!(matches!(err, PolicyError::NotADirectory(_)));
    }

    #[test]
    fn empty_files_yield_empty_policy() {
        let dir = tempfile::tempdir().unwrap();
        let root = write_repo(&dir, "# nothing\n\n", "");
        let policy = RepoPolicy::load(&root).unwrap();
        assert!(policy.trusted.is_empty());
        assert!(policy.sensitives.is_empty());
    }
}
