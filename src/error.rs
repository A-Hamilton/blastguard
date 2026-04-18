//! Unified error type for the library surface.
//!
//! `anyhow::Result` is used at binary boundaries (`src/main.rs`); library modules
//! return [`Result`] aliased to `std::result::Result<T, BlastGuardError>`.

use std::path::PathBuf;

/// All errors produced by the `BlastGuard` library. Variants carry enough context
/// to convert to an MCP `CallToolResult { is_error: true, .. }` at the tool boundary.
#[derive(Debug, thiserror::Error)]
pub enum BlastGuardError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("tree-sitter parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("ambiguous old_text in {path}: {count} matches at lines {lines:?}")]
    AmbiguousEdit {
        path: PathBuf,
        count: usize,
        lines: Vec<u32>,
    },

    #[error("old_text not found in {path}; closest match at line {line} ({similarity:.0}% similar): {fragment}")]
    EditNotFound {
        path: PathBuf,
        line: u32,
        similarity: f32,
        fragment: String,
    },

    #[error("no test runner detected; pass --test-command to override")]
    NoTestRunner,

    #[error("test runner timed out after {seconds}s")]
    TestTimeout { seconds: u64 },

    #[error("test runner crashed: {stderr}")]
    TestCrashed { stderr: String },

    #[error("cache deserialization failed: {0}")]
    CacheCorrupt(String),

    #[error("config error: {0}")]
    Config(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for BlastGuardError {
    fn from(source: std::io::Error) -> Self {
        Self::Io {
            path: PathBuf::new(),
            source,
        }
    }
}

/// Library-scoped result alias.
pub type Result<T> = std::result::Result<T, BlastGuardError>;
