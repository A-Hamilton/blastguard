//! Import path resolution — SPEC §6.
//!
//! TS/JS follow the extension/index ladder, honouring `tsconfig.json`
//! `compilerOptions.paths` and `baseUrl`. Python resolves via the package tree.
//! Rust uses the `mod` hierarchy rooted at `src/`.

use std::path::PathBuf;

/// Result of import resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveResult {
    /// Resolves to a file inside the project.
    Internal(PathBuf),
    /// Resolves to an external package. `symbols` is the list of names
    /// brought into the importing file (used by `libraries` dispatcher).
    External {
        library: String,
        symbols: Vec<String>,
    },
    /// Could not resolve. Downgraded to [`crate::graph::types::Confidence::Inferred`]
    /// rather than being dropped (SPEC §6.5).
    Unresolved,
}

// TODO(phase-1.3): load_tsconfig(project_root), resolve_ts, resolve_py, resolve_rs.
