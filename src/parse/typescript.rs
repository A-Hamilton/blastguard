//! TypeScript driver — tree-sitter-typescript.
//!
//! Phase 1.2 emits function / class / method / interface / type-alias
//! symbols, forward `Calls` + `Imports` + `Implements` edges, and
//! resolved/unresolved `LibraryImport`s.

use std::path::Path;

use super::ParseOutput;

/// Placeholder — returns an empty parse until Phase 1.2 lands.
#[must_use]
pub fn extract(_path: &Path, _source: &str) -> ParseOutput {
    // TODO(phase-1.2): implement with `queries/typescript.scm`.
    ParseOutput::default()
}
