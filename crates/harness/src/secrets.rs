//! Secret store: secrets live in memory only, loaded at startup from the
//! environment or a local user config file (read-only). The harness never
//! writes them to disk and never passes them to child processes (env/argv).
//! Any output sent outside goes through `mask`, which replaces every
//! occurrence of a known secret with `***`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use thiserror::Error;

const MASK: &str = "***";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("environment variable {0} is not set")]
    EnvVarMissing(String),
    #[error("cannot read secrets file {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("malformed line {line} in {path}: expected NAME=value")]
    MalformedLine { path: PathBuf, line: usize },
}

#[derive(Debug, Clone)]
struct SecretEntry {
    value: String,
    expires_at: Option<SystemTime>,
}

#[derive(Debug, Default, Clone)]
pub struct SecretStore {
    entries: HashMap<String, SecretEntry>,
}

impl SecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads the given environment variables into the store. Fail-closed: a
    /// missing variable is an error, never a partially filled store.
    pub fn from_env(names: &[&str]) -> Result<Self, SecretError> {
        let mut store = Self::new();
        for name in names {
            let value =
                std::env::var(name).map_err(|_| SecretError::EnvVarMissing((*name).to_string()))?;
            store.insert(*name, value, None);
        }
        Ok(store)
    }

    /// Loads secrets from a local `NAME=value` config file (one per line,
    /// blank lines and `#` comments ignored). Read-only: the harness never
    /// writes this file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path).map_err(|source| SecretError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;
        let mut store = Self::new();
        for (idx, raw) in content.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let malformed = || SecretError::MalformedLine {
                path: path.to_path_buf(),
                line: idx + 1,
            };
            let (name, value) = line.split_once('=').ok_or_else(malformed)?;
            let name = name.trim();
            if name.is_empty() {
                return Err(malformed());
            }
            store.insert(name, value.trim(), None);
        }
        Ok(store)
    }

    /// Inserts or replaces a secret, with an optional time-to-live. Refreshing
    /// a secret (e.g. a GitHub App token valid for 1 h) is a re-insert with a
    /// new value and TTL.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        ttl: Option<Duration>,
    ) {
        let expires_at = ttl.map(|d| SystemTime::now() + d);
        self.entries.insert(
            name.into(),
            SecretEntry {
                value: value.into(),
                expires_at,
            },
        );
    }

    /// Returns the secret value, or `None` if it is unknown or expired.
    pub fn get(&self, name: &str) -> Option<&str> {
        let entry = self.entries.get(name)?;
        if Self::entry_expired(entry) {
            return None;
        }
        Some(&entry.value)
    }

    pub fn is_expired(&self, name: &str) -> bool {
        self.entries.get(name).is_some_and(Self::entry_expired)
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replaces every occurrence of every known secret value (including
    /// expired ones — an old value may have leaked into logs) with `***`.
    /// Empty values are never masked; longer values are replaced first so a
    /// secret that contains another secret is not truncated.
    pub fn mask(&self, text: &str) -> String {
        let mut values: Vec<&str> = self
            .entries
            .values()
            .map(|e| e.value.as_str())
            .filter(|v| !v.is_empty())
            .collect();
        values.sort_by_key(|v| std::cmp::Reverse(v.len()));
        let mut out = text.to_string();
        for v in values {
            out = out.replace(v, MASK);
        }
        out
    }

    /// Returns true if `text` contains any known secret value (including
    /// expired ones). Used by the subprocess launcher to fail-closed before
    /// passing anything to a child process.
    pub fn contains_value(&self, text: &str) -> bool {
        self.entries
            .values()
            .any(|e| !e.value.is_empty() && text.contains(&e.value))
    }

    fn entry_expired(entry: &SecretEntry) -> bool {
        matches!(entry.expires_at, Some(t) if SystemTime::now() >= t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn insert_and_get_roundtrip() {
        let mut store = SecretStore::new();
        assert!(store.is_empty());
        store.insert("token", "abc123", None);
        assert_eq!(store.get("token"), Some("abc123"));
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
        assert!(!store.is_expired("token"));
    }

    #[test]
    fn get_unknown_name_returns_none() {
        let store = SecretStore::new();
        assert_eq!(store.get("nope"), None);
        assert!(!store.is_expired("nope"));
    }

    #[test]
    fn expired_secret_is_not_returned() {
        let mut store = SecretStore::new();
        store.insert("token", "abc123", Some(Duration::from_secs(0)));
        assert_eq!(store.get("token"), None);
        assert!(store.is_expired("token"));
    }

    #[test]
    fn live_ttl_secret_is_returned() {
        let mut store = SecretStore::new();
        store.insert("token", "abc123", Some(Duration::from_secs(3600)));
        assert_eq!(store.get("token"), Some("abc123"));
        assert!(!store.is_expired("token"));
    }

    #[test]
    fn refresh_replaces_value_and_ttl() {
        let mut store = SecretStore::new();
        store.insert("token", "old", Some(Duration::from_secs(0)));
        assert!(store.is_expired("token"));
        store.insert("token", "new", Some(Duration::from_secs(3600)));
        assert_eq!(store.get("token"), Some("new"));
        assert!(!store.is_expired("token"));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_deletes_entry() {
        let mut store = SecretStore::new();
        store.insert("token", "abc123", None);
        assert!(store.remove("token"));
        assert!(!store.remove("token"));
        assert_eq!(store.get("token"), None);
    }

    #[test]
    fn from_env_loads_present_variables() {
        std::env::set_var("LEHARNESS_TEST_ENV_A", "va");
        std::env::set_var("LEHARNESS_TEST_ENV_B", "vb");
        let store =
            SecretStore::from_env(&["LEHARNESS_TEST_ENV_A", "LEHARNESS_TEST_ENV_B"]).unwrap();
        assert_eq!(store.get("LEHARNESS_TEST_ENV_A"), Some("va"));
        assert_eq!(store.get("LEHARNESS_TEST_ENV_B"), Some("vb"));
    }

    #[test]
    fn from_env_missing_variable_is_fail_closed() {
        std::env::remove_var("LEHARNESS_TEST_ENV_MISSING");
        let result = SecretStore::from_env(&["LEHARNESS_TEST_ENV_MISSING"]);
        assert!(
            matches!(result, Err(SecretError::EnvVarMissing(name)) if name == "LEHARNESS_TEST_ENV_MISSING")
        );
    }

    #[test]
    fn from_file_parses_name_value_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.txt");
        std::fs::write(&path, "# comment\n\nTOKEN=abc=123\nOTHER=xyz\n").unwrap();
        let store = SecretStore::from_file(&path).unwrap();
        assert_eq!(store.get("TOKEN"), Some("abc=123"));
        assert_eq!(store.get("OTHER"), Some("xyz"));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn from_file_rejects_malformed_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.txt");
        std::fs::write(&path, "TOKEN=ok\nnot-a-pair\n").unwrap();
        let result = SecretStore::from_file(&path);
        assert!(matches!(
            result,
            Err(SecretError::MalformedLine { line: 2, .. })
        ));
    }

    #[test]
    fn from_file_missing_file_is_read_failed() {
        let result = SecretStore::from_file("/nonexistent/le-harness-secrets.txt");
        assert!(matches!(result, Err(SecretError::ReadFailed { .. })));
    }

    #[test]
    fn mask_replaces_every_occurrence() {
        let mut store = SecretStore::new();
        store.insert("token", "s3cr3t", None);
        assert_eq!(
            store.mask("leak s3cr3t puis encore s3cr3t!"),
            "leak *** puis encore ***!"
        );
    }

    #[test]
    fn mask_replaces_longer_secret_first() {
        let mut store = SecretStore::new();
        store.insert("short", "abc", None);
        store.insert("long", "abcd", None);
        assert_eq!(store.mask("xabcdy"), "x***y");
    }

    #[test]
    fn mask_ignores_empty_values() {
        let mut store = SecretStore::new();
        store.insert("empty", "", None);
        assert_eq!(store.mask("hello world"), "hello world");
    }

    #[test]
    fn mask_still_masks_expired_secrets() {
        let mut store = SecretStore::new();
        store.insert("token", "oldtok", Some(Duration::from_secs(0)));
        assert!(store.is_expired("token"));
        assert_eq!(store.mask("token was oldtok"), "token was ***");
    }

    #[test]
    fn mask_without_secrets_returns_text_unchanged() {
        let store = SecretStore::new();
        assert_eq!(store.mask("rien a cacher"), "rien a cacher");
    }

    #[test]
    fn contains_value_detects_known_secrets() {
        let mut store = SecretStore::new();
        store.insert("token", "s3cr3t", None);
        store.insert("empty", "", None);
        assert!(store.contains_value("leak: s3cr3t!"));
        assert!(!store.contains_value("nothing here"));
        assert!(!store.contains_value(""));
        let empty_only = SecretStore::new();
        assert!(!empty_only.contains_value("anything"));
    }

    proptest! {
        #[test]
        fn masked_output_never_contains_the_secret(
            secret in "[^\\x00]{1,64}".prop_filter(
                "secret must contain at least one non-* char (mask is ***)",
                |s| s.chars().any(|c| c != '*')
            ),
            text in ".*",
        ) {
            let mut store = SecretStore::new();
            store.insert("k", secret.clone(), None);
            let masked = store.mask(&text);
            prop_assert!(!masked.contains(&secret));
        }
    }
}
