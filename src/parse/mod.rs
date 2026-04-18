//! Tree-sitter driven parsers per language — SPEC §8.
//!
//! Phase 1.2 wires TS/JS/PY/RS. Each language module exposes a pure function
//! `extract(path, source) -> ParseOutput` that returns symbols + edges + a
//! `partial_parse` flag for graceful degradation (SPEC §8.5).

pub mod body_hash;
pub mod javascript;
pub mod python;
pub mod resolve;
pub mod rust;
pub mod symbols;
pub mod typescript;

use std::path::Path;

use crate::graph::types::{Edge, LibraryImport, Symbol};

/// Output of a single-file parse pass.
#[derive(Debug, Default)]
pub struct ParseOutput {
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
    pub library_imports: Vec<LibraryImport>,
    /// Set when tree-sitter reported ERROR nodes but we recovered partial
    /// information (SPEC §8.5).
    pub partial_parse: bool,
}

/// Dispatch by file extension. Unsupported extensions return [`None`] and are
/// indexed by grep only (SPEC §13).
#[must_use]
pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?;
    match ext {
        "ts" | "tsx" | "mts" | "cts" => Some(Language::TypeScript),
        "js" | "jsx" | "mjs" | "cjs" => Some(Language::JavaScript),
        "py" | "pyi" => Some(Language::Python),
        "rs" => Some(Language::Rust),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    TypeScript,
    JavaScript,
    Python,
    Rust,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detect_language_by_extension() {
        assert_eq!(
            detect_language(&PathBuf::from("a.ts")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            detect_language(&PathBuf::from("a.tsx")),
            Some(Language::TypeScript)
        );
        assert_eq!(
            detect_language(&PathBuf::from("a.py")),
            Some(Language::Python)
        );
        assert_eq!(
            detect_language(&PathBuf::from("a.rs")),
            Some(Language::Rust)
        );
        assert_eq!(detect_language(&PathBuf::from("a.md")), None);
        assert_eq!(detect_language(&PathBuf::from("Makefile")), None);
    }
}
