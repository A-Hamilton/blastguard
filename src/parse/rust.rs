//! Rust driver — tree-sitter-rust.

use std::path::Path;

use super::ParseOutput;

#[must_use]
pub fn extract(_path: &Path, _source: &str) -> ParseOutput {
    // TODO(phase-1.2): implement with `queries/rust.scm`.
    ParseOutput::default()
}
