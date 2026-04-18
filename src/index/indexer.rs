//! Parallel indexer — SPEC §10.
//!
//! Cold target: <3s for 10K files. Warm target: <500ms via BLAKE3 Merkle
//! skip of unchanged subtrees.

use std::path::{Path, PathBuf};

use crate::graph::types::CodeGraph;
use crate::parse::detect_language;
use crate::Result;

/// Walk `project_root` respecting `.gitignore`, returning only files that
/// one of the language drivers can parse (via [`detect_language`]).
///
/// Uses the `ignore` crate's standard filters: hidden files and directories
/// (`.git/`, `.github/`, etc.) are skipped, and `.gitignore` rules are
/// respected. `.gitignore` is only active when `project_root` is inside a
/// git repository, which is the normal `BlastGuard` deployment scenario.
#[must_use]
pub fn walk_project(project_root: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(project_root)
        .standard_filters(true)
        .build()
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .filter(|e| detect_language(e.path()).is_some())
        .map(ignore::DirEntry::into_path)
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn mk(dir: &std::path::Path, files: &[&str]) {
        for rel in files {
            let full = dir.join(rel);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            fs::write(&full, "").expect("write");
        }
    }

    #[test]
    fn walks_source_files_and_respects_gitignore() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // `ignore` crate only reads .gitignore when inside a git repo.
        std::process::Command::new("git")
            .args(["-C", tmp.path().to_str().unwrap(), "init", "-q"])
            .status()
            .expect("git init");
        fs::write(tmp.path().join(".gitignore"), "node_modules/\ntarget/\n").expect("gitignore");
        mk(tmp.path(), &[
            "src/a.ts",
            "src/b.py",
            "src/c.rs",
            "src/d.js",
            "node_modules/skip.ts",
            "target/build.rs",
            "README.md",
            "Cargo.toml",
        ]);
        let files = walk_project(tmp.path());
        let rels: Vec<std::path::PathBuf> = files
            .iter()
            .filter_map(|f| f.strip_prefix(tmp.path()).ok().map(std::path::Path::to_path_buf))
            .collect();
        assert!(rels.iter().any(|p| p.ends_with("a.ts")), "missing a.ts: {rels:?}");
        assert!(rels.iter().any(|p| p.ends_with("b.py")));
        assert!(rels.iter().any(|p| p.ends_with("c.rs")));
        assert!(rels.iter().any(|p| p.ends_with("d.js")));
        assert!(!rels.iter().any(|p| p.components().any(|c| c.as_os_str() == "node_modules")),
            "node_modules should be excluded: {rels:?}");
        assert!(!rels.iter().any(|p| p.components().any(|c| c.as_os_str() == "target")));
        assert!(!rels.iter().any(|p| p.ends_with("README.md")));
        assert!(!rels.iter().any(|p| p.ends_with("Cargo.toml")));
    }

    #[test]
    fn walks_nested_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        mk(tmp.path(), &[
            "src/deep/nested/x.ts",
            "src/deep/y.py",
        ]);
        let files = walk_project(tmp.path());
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("x.ts")));
        assert!(files.iter().any(|f| f.ends_with("y.py")));
    }

    #[test]
    fn empty_project_returns_empty_vec() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert!(walk_project(tmp.path()).is_empty());
    }
}
