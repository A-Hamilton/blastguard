//! Parallel indexer — SPEC §10.
//!
//! Cold target: <3s for 10K files. Warm target: <500ms via BLAKE3 Merkle
//! skip of unchanged subtrees.

use std::path::Path;

use crate::graph::types::CodeGraph;
use crate::Result;

/// Cold-index a project from scratch. Ignores the cache.
///
/// # Errors
/// Surfaces I/O errors encountered while walking or reading source files.
#[must_use = "cold index result should be used or persisted"]
pub fn cold_index(_project_root: &Path) -> Result<CodeGraph> {
    // TODO(phase-1.4): walk with `ignore`, hash with BLAKE3, parse with rayon.
    Ok(CodeGraph::new())
}

/// Warm-start: load the cache, compute current hashes in parallel, skip
/// unchanged subtrees via `tree_hashes`, reparse only changed files.
///
/// # Errors
/// Returns an error if the cache is corrupt (caller should fall back to
/// [`cold_index`]).
#[must_use = "warm start result should be used"]
pub fn warm_start(_project_root: &Path) -> Result<CodeGraph> {
    // TODO(phase-1.4): load cache, Merkle-diff, incremental reparse.
    Ok(CodeGraph::new())
}
