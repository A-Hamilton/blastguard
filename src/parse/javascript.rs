//! JavaScript driver — tree-sitter-javascript.
//!
//! Shares most queries with [`super::typescript`] minus type annotations.

use std::path::Path;

use super::ParseOutput;

#[must_use]
pub fn extract(_path: &Path, _source: &str) -> ParseOutput {
    // TODO(phase-1.2): implement with `queries/javascript.scm`.
    ParseOutput::default()
}
