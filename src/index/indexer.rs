//! Parallel indexer — SPEC §10.
//!
//! Cold target: <3s for 10K files. Warm target: <500ms via BLAKE3 Merkle
//! skip of unchanged subtrees.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::graph::types::CodeGraph;
use crate::parse::{detect_language, Language, ParseOutput};
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
/// Persists a cache after building the graph so that subsequent runs can use
/// [`warm_start`] instead of re-parsing everything. Cache persistence failures
/// are logged via `tracing::warn` rather than propagated — a failure here does
/// not affect the returned graph.
///
/// # Errors
/// Surfaces unreadable files as logged warnings rather than failing the whole
/// index. A file that tree-sitter cannot parse at all is returned as
/// [`ParseOutput::default`] with `partial_parse = true` and contributes no
/// symbols.
#[must_use = "cold index result should be used or persisted"]
pub fn cold_index(project_root: &Path) -> Result<CodeGraph> {
    let files = walk_project(project_root);

    let parses: Vec<ParseOutput> = files
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

    // Resolve internal Imports edges (crate::/./) to real file paths so
    // callers_of / importers_of can cross files.
    crate::parse::resolve::resolve_imports(&mut graph, project_root);

    // Then try to pin each Unresolved Calls edge at a unique function in the
    // from-file's imports — upgrades to Inferred when exactly one candidate
    // exists. Relies on resolve_imports having run first.
    crate::parse::resolve::resolve_calls(&mut graph);

    // Persist the fresh cache so the next run can warm-start. Best-effort:
    // a write failure should not cause the caller's index to fail.
    let mut file_hashes = HashMap::new();
    for path in &files {
        match crate::index::cache::hash_file(path) {
            Ok(h) => {
                file_hashes.insert(path.clone(), h);
            }
            Err(err) => {
                tracing::warn!(path = %path.display(), error = %err, "hash_file failed during cache persist");
            }
        }
    }
    let mut tree_hashes = HashMap::new();
    match crate::index::cache::hash_project_tree(project_root, walk_project) {
        Ok(root_hash) => {
            tree_hashes.insert(project_root.to_path_buf(), root_hash);
        }
        Err(err) => {
            tracing::warn!(error = %err, "hash_project_tree failed during cache persist");
        }
    }
    let cache = crate::index::cache::CacheFile {
        version: crate::index::cache::CACHE_VERSION,
        file_hashes,
        tree_hashes,
        graph: graph.clone(),
        tsconfig: None,
    };
    let cache_path = project_root.join(".blastguard").join("cache.bin");
    if let Err(err) = crate::index::cache::save(&cache_path, &cache) {
        tracing::warn!(error = %err, "failed to persist cache after cold index");
    }

    Ok(graph)
}

/// Warm-start: load the cache, compute current hashes in parallel, skip
/// unchanged files via BLAKE3 Merkle comparison, reparse only changed files.
///
/// # Fast path
/// If the root directory Merkle hash matches the cached value, the entire
/// project tree is unchanged and the cached graph is returned verbatim.
///
/// # Slow path
/// Per-file hashes are compared to the cache. Changed files are dropped from
/// the graph via [`CodeGraph::remove_file`] and then re-parsed. Deleted files
/// are also dropped. The updated cache is persisted before returning.
///
/// # Fallback
/// If no cache exists (first run after `cold_index` is skipped) or the cache
/// is corrupt, falls through to [`cold_index`].
///
/// # Errors
/// Returns an error if the cache is corrupt and unrecoverable, or if
/// filesystem I/O fails during hashing.
#[must_use = "warm start result should be used"]
pub fn warm_start(project_root: &Path) -> Result<CodeGraph> {
    let cache_path = project_root.join(".blastguard").join("cache.bin");
    let Some(cache) = crate::index::cache::load(&cache_path)? else {
        return cold_index(project_root);
    };

    // Fast path: if the gitignore-filtered Merkle hash matches, the entire
    // tracked tree is unchanged — skip file-level work entirely.
    let current_root_hash = crate::index::cache::hash_project_tree(project_root, walk_project)?;
    if cache.tree_hashes.get(project_root).copied() == Some(current_root_hash) {
        return Ok(cache.graph);
    }

    // Slow path: walk the project, hash each current file, compare to the
    // cached file_hashes map, reparse only files whose hashes differ.
    let files = walk_project(project_root);
    let mut graph = cache.graph;
    let mut current_hashes: HashMap<PathBuf, u64> = HashMap::new();
    let mut changed: Vec<PathBuf> = Vec::new();

    // Hash files in parallel (SPEC §10) and skip any that disappear mid-walk
    // (race with external git checkout or watcher delete) rather than crashing.
    let hash_results: Vec<(PathBuf, crate::Result<u64>)> = files
        .par_iter()
        .map(|path| (path.clone(), crate::index::cache::hash_file(path)))
        .collect();

    for (path, res) in hash_results {
        match res {
            Ok(h) => {
                if cache.file_hashes.get(&path).copied() != Some(h) {
                    changed.push(path.clone());
                }
                current_hashes.insert(path, h);
            }
            Err(err) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %err,
                    "skipping file — hash failed during warm_start"
                );
            }
        }
    }

    // Drop stale file entries (deleted or renamed files no longer in walk).
    for cached_path in cache.file_hashes.keys() {
        if !current_hashes.contains_key(cached_path) {
            graph.remove_file(cached_path);
        }
    }

    // Drop changed files before reparsing so stale symbols are removed.
    for path in &changed {
        graph.remove_file(path);
    }

    let reparses: Vec<ParseOutput> = changed
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            let lang = detect_language(path)?;
            let out = match lang {
                Language::TypeScript => crate::parse::typescript::extract(path, &source),
                Language::JavaScript => crate::parse::javascript::extract(path, &source),
                Language::Python => crate::parse::python::extract(path, &source),
                Language::Rust => crate::parse::rust::extract(path, &source),
            };
            Some(out)
        })
        .collect();

    let mut restitch_files: Vec<PathBuf> = Vec::new();
    for out in reparses {
        // Track which files we re-inserted so we can rebuild their
        // reverse_edges below — other files' forward edges are kept
        // dangling by remove_file for ORPHAN detection.
        if let Some(sym) = out.symbols.first() {
            restitch_files.push(sym.id.file.clone());
        }
        for sym in out.symbols {
            graph.insert_symbol(sym);
        }
        for edge in out.edges {
            graph.insert_edge(edge);
        }
        graph.library_imports.extend(out.library_imports);
    }

    // New files may have introduced unresolved Imports edges during reparse.
    // Cheaper than persisting the resolved state across warm starts and
    // keeps resolver logic in one place.
    crate::parse::resolve::resolve_imports(&mut graph, project_root);
    crate::parse::resolve::resolve_calls(&mut graph);
    for file in &restitch_files {
        graph.restitch_reverse_edges_for_file(file);
    }

    // Persist the updated cache.
    let mut tree_hashes = HashMap::new();
    tree_hashes.insert(project_root.to_path_buf(), current_root_hash);
    let fresh = crate::index::cache::CacheFile {
        version: crate::index::cache::CACHE_VERSION,
        file_hashes: current_hashes,
        tree_hashes,
        graph: graph.clone(),
        tsconfig: cache.tsconfig,
    };
    if let Err(err) = crate::index::cache::save(&cache_path, &fresh) {
        tracing::warn!(error = %err, "failed to persist updated cache after warm start");
    }

    Ok(graph)
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
        std::fs::write(tmp.path().join("src/b.py"), "def bar():\n    return 1\n").expect("write");
        std::fs::write(tmp.path().join("src/c.rs"), "pub fn baz() -> i32 { 1 }").expect("write");

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
        std::fs::write(tmp.path().join("src/a.ts"), "export function included() {}")
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
        mk(
            tmp.path(),
            &[
                "src/a.ts",
                "src/b.py",
                "src/c.rs",
                "src/d.js",
                "node_modules/skip.ts",
                "target/build.rs",
                "README.md",
                "Cargo.toml",
            ],
        );
        let files = walk_project(tmp.path());
        let rels: Vec<std::path::PathBuf> = files
            .iter()
            .filter_map(|f| {
                f.strip_prefix(tmp.path())
                    .ok()
                    .map(std::path::Path::to_path_buf)
            })
            .collect();
        assert!(
            rels.iter().any(|p| p.ends_with("a.ts")),
            "missing a.ts: {rels:?}"
        );
        assert!(rels.iter().any(|p| p.ends_with("b.py")));
        assert!(rels.iter().any(|p| p.ends_with("c.rs")));
        assert!(rels.iter().any(|p| p.ends_with("d.js")));
        assert!(
            !rels
                .iter()
                .any(|p| p.components().any(|c| c.as_os_str() == "node_modules")),
            "node_modules should be excluded: {rels:?}"
        );
        assert!(!rels
            .iter()
            .any(|p| p.components().any(|c| c.as_os_str() == "target")));
        assert!(!rels.iter().any(|p| p.ends_with("README.md")));
        assert!(!rels.iter().any(|p| p.ends_with("Cargo.toml")));
    }

    #[test]
    fn walks_nested_directories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        mk(tmp.path(), &["src/deep/nested/x.ts", "src/deep/y.py"]);
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

    #[test]
    fn cold_index_persists_cache_for_next_warm_start() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src/a.ts"), "export function foo() {}").expect("write");

        let _ = cold_index(tmp.path()).expect("cold_index");
        let cache_path = tmp.path().join(".blastguard").join("cache.bin");
        assert!(
            cache_path.is_file(),
            "cold_index should persist the cache at {}",
            cache_path.display()
        );
    }

    #[test]
    fn warm_start_returns_cached_graph_when_tree_unchanged() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src/a.ts"), "export function foo() {}").expect("write");

        // Prime the cache.
        let _ = cold_index(tmp.path()).expect("cold_index");

        // Warm start — no files changed.
        let graph = warm_start(tmp.path()).expect("warm_start");
        assert!(
            graph.symbols.keys().any(|id| id.name == "foo"),
            "expected cached symbol to survive warm_start: {:?}",
            graph.symbols.keys().map(|k| &k.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn warm_start_reparses_changed_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src/a.ts"), "export function foo() {}").expect("write");

        let _ = cold_index(tmp.path()).expect("cold_index");

        // Modify the file.
        std::fs::write(tmp.path().join("src/a.ts"), "export function bar() {}").expect("rewrite");

        let graph = warm_start(tmp.path()).expect("warm_start");
        assert!(
            graph.symbols.keys().any(|id| id.name == "bar"),
            "new symbol missing after warm_start reparse: {:?}",
            graph.symbols.keys().map(|k| &k.name).collect::<Vec<_>>()
        );
        assert!(
            !graph.symbols.keys().any(|id| id.name == "foo"),
            "old symbol survived warm_start reparse: {:?}",
            graph.symbols.keys().map(|k| &k.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn warm_start_fast_path_survives_gitignored_file_changes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .expect("git init");
        std::fs::write(tmp.path().join(".gitignore"), "node_modules/\n").expect("write gitignore");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir src");
        std::fs::create_dir_all(tmp.path().join("node_modules")).expect("mkdir nm");
        std::fs::write(tmp.path().join("src/a.ts"), "export function foo() {}").expect("write src");

        // Prime the cache.
        let _ = cold_index(tmp.path()).expect("cold_index");

        // Change something inside node_modules — gitignored, should NOT invalidate.
        std::fs::write(
            tmp.path().join("node_modules/installed.ts"),
            "export function x() {}",
        )
        .expect("write ignored");
        std::fs::write(
            tmp.path().join("node_modules/installed.ts"),
            "export function x() { return 1; }",
        )
        .expect("mutate");

        let graph = warm_start(tmp.path()).expect("warm_start");
        assert!(
            graph.symbols.keys().any(|id| id.name == "foo"),
            "foo should still be present — tree-hash check should have fast-pathed"
        );
        assert!(
            !graph.symbols.keys().any(|id| id.name == "x"),
            "x is in node_modules/ and should not be indexed"
        );
    }

    #[test]
    fn warm_start_without_cache_falls_back_to_cold_index() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src/a.ts"), "export function foo() {}").expect("write");

        // No cold_index first — cache is absent.
        let graph = warm_start(tmp.path()).expect("warm_start");
        assert!(
            graph.symbols.keys().any(|id| id.name == "foo"),
            "warm_start with no cache should fall back to cold_index"
        );
    }
}
