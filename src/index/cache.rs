//! On-disk cache at `.blastguard/cache.bin` — SPEC §9.
//!
//! Format: rmp-serde. Keyed by BLAKE3 file + subtree hashes so we skip
//! unchanged directories on warm start.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::graph::types::CodeGraph;

/// Bump when the serialised schema changes in an incompatible way.
/// Drop + rebuild on mismatch (SPEC §9).
pub const CACHE_VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CacheFile {
    pub version: u32,
    pub file_hashes: HashMap<PathBuf, u64>,
    pub tree_hashes: HashMap<PathBuf, u64>,
    pub graph: CodeGraph,
    pub tsconfig: Option<TsConfigSnapshot>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TsConfigSnapshot {
    pub base_url: Option<PathBuf>,
    pub paths: HashMap<String, Vec<String>>,
}

// TODO(phase-1.4): load(path), save(path, &CacheFile), BLAKE3 merkle helpers.
