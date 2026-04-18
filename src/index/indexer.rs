//! Parallel indexer — SPEC §10.
//!
//! Cold target: <3s for 10K files. Warm target: <500ms via BLAKE3 Merkle
//! skip of unchanged subtrees.

use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::graph::types::CodeGraph;
use crate::parse::{detect_language, Language};
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

/// Cold-index a project from scratch. Ignores any existing cache.
///
/// Walks the project respecting `.gitignore`, dispatches parsing across rayon
/// workers (each worker uses its own thread-local tree-sitter parser), and
/// assembles a fresh [`CodeGraph`]. Target: under 3s for 10K files.
///
/// # Errors
/// Surfaces unreadable files as logged warnings rather than failing the whole
/// index. A file that tree-sitter cannot parse at all is returned as
/// [`ParseOutput::default`] with `partial_parse = true` and contributes no
/// symbols.
#[must_use = "cold index result should be used or persisted"]
pub fn cold_index(project_root: &Path) -> Result<CodeGraph> {
    let files = walk_project(project_root);

    let parses: Vec<crate::parse::ParseOutput> = files
        .par_iter()
        .filter_map(|path| match std::fs::read_to_string(path) {
            Ok(source) => {
                let lang = detect_language(path)?;
                let out = match lang {
                    Language::TypeScript => crate::parse::typescript::extract(path, &source),
                    Language::JavaScript => crate::parse::javascript::extract(path, &source),
                    Language::Python => crate::parse::python::extract(path, &source),
                    Language::Rust => crate::parse::rust::extract(path, &source),
                };
                Some(out)
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "skipping file — read error during cold index"
                );
                None
            }
        })
        .collect();

    let mut graph = CodeGraph::new();
    for out in parses {
        for sym in out.symbols {
            graph.insert_symbol(sym);
        }
        for edge in out.edges {
            graph.insert_edge(edge);
        }
        graph.library_imports.extend(out.library_imports);
    }
    Ok(graph)
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

    #[test]
    fn cold_index_extracts_symbols_across_ts_py_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(
            tmp.path().join("src/a.ts"),
            "export function foo() { return 1; }",
        )
        .expect("write");
        std::fs::write(
            tmp.path().join("src/b.py"),
            "def bar():\n    return 1\n",
        )
        .expect("write");
        std::fs::write(
            tmp.path().join("src/c.rs"),
            "pub fn baz() -> i32 { 1 }",
        )
        .expect("write");

        let graph = cold_index(tmp.path()).expect("cold_index");
        assert!(
            graph.symbols.keys().any(|id| id.name == "foo"),
            "missing foo; symbols: {:?}",
            graph.symbols.keys().map(|k| &k.name).collect::<Vec<_>>()
        );
        assert!(graph.symbols.keys().any(|id| id.name == "bar"));
        assert!(graph.symbols.keys().any(|id| id.name == "baz"));
    }

    #[test]
    fn cold_index_respects_gitignore() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // Need a git init for .gitignore to activate — mirror walk_project tests.
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .expect("git init");
        std::fs::write(tmp.path().join(".gitignore"), "vendor/\n").expect("gitignore");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir src");
        std::fs::create_dir_all(tmp.path().join("vendor")).expect("mkdir vendor");
        std::fs::write(
            tmp.path().join("src/a.ts"),
            "export function included() {}",
        )
        .expect("write");
        std::fs::write(
            tmp.path().join("vendor/skipped.ts"),
            "export function excluded() {}",
        )
        .expect("write");

        let graph = cold_index(tmp.path()).expect("cold_index");
        assert!(graph.symbols.keys().any(|id| id.name == "included"));
        assert!(
            !graph.symbols.keys().any(|id| id.name == "excluded"),
            "vendor/ should be gitignore'd"
        );
    }

    #[test]
    fn cold_index_empty_project_returns_empty_graph() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let graph = cold_index(tmp.path()).expect("cold_index");
        assert!(graph.symbols.is_empty());
        assert!(graph.library_imports.is_empty());
    }

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
