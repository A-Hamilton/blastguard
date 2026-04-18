//! Project-level configuration loaded from `.blastguard/config.toml`.
//!
//! Phase 1 surface is minimal: an optional override for the test command.
//! Everything else is derived from project files (tsconfig, package.json, …).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{BlastGuardError, Result};

/// On-disk schema for `.blastguard/config.toml`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Config {
    /// Overrides runner auto-detection. Example: `"pytest -x --tb=short"`.
    #[serde(default)]
    pub test_command: Option<String>,

    /// Watcher debounce window in milliseconds (default 100).
    #[serde(default)]
    pub watcher_debounce_ms: Option<u64>,
}

impl Config {
    /// Load config from `<project_root>/.blastguard/config.toml`, returning
    /// [`Config::default`] when the file does not exist.
    ///
    /// # Errors
    /// Returns [`BlastGuardError::Config`] on malformed TOML.
    #[must_use = "callers should propagate or handle config load errors"]
    pub fn load(project_root: &Path) -> Result<Self> {
        let path = project_root.join(".blastguard").join("config.toml");
        if !path.exists() {
            return Ok(Self::default());
        }
        let body = std::fs::read_to_string(&path).map_err(|source| BlastGuardError::Io {
            path: path.clone(),
            source,
        })?;
        toml::from_str(&body).map_err(|e| BlastGuardError::Config(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_default() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg = Config::load(tmp.path()).expect("load default");
        assert!(cfg.test_command.is_none());
        assert!(cfg.watcher_debounce_ms.is_none());
    }

    #[test]
    fn parses_overrides() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join(".blastguard");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("config.toml"),
            "test_command = \"pytest -x\"\nwatcher_debounce_ms = 250\n",
        )
        .expect("write");
        let cfg = Config::load(tmp.path()).expect("load");
        assert_eq!(cfg.test_command.as_deref(), Some("pytest -x"));
        assert_eq!(cfg.watcher_debounce_ms, Some(250));
    }
}
