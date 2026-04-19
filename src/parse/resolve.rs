//! Import path resolution — SPEC §6.
//!
//! TS/JS follow the extension/index ladder, honouring `tsconfig.json`
//! `compilerOptions.paths` and `baseUrl` (Task 9). Python resolves via the
//! package tree (Task 10). Rust uses the `mod` hierarchy rooted at `src/`
//! (Task 11).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{BlastGuardError, Result};
use crate::graph::types::{CodeGraph, Confidence, EdgeKind, SymbolId};

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

/// Parsed `tsconfig.json` `compilerOptions` subset relevant to resolution.
/// Task 9 populates this via [`load_tsconfig`]; Task 8 only reads it.
#[derive(Debug, Default, Clone)]
pub struct TsConfig {
    /// `compilerOptions.baseUrl` resolved to an absolute path.
    pub base_url: Option<PathBuf>,
    /// `compilerOptions.paths` alias map.
    pub paths: HashMap<String, Vec<String>>,
}

/// Resolve a TypeScript/JavaScript import specifier.
///
/// Ladder:
/// 1. If a `tsconfig` is provided, consult [`resolve_via_tsconfig`] (Task 9).
/// 2. Bare specifiers (not starting with `.` or `/`) → [`ResolveResult::External`].
///    Scoped packages (`@scope/pkg`) keep both segments as the library name.
/// 3. Relative specifiers → try `.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`
///    then `/index.{ts,tsx,js,jsx}`.
/// 4. No match → [`ResolveResult::Unresolved`].
#[must_use]
pub fn resolve_ts(
    project_root: &Path,
    from_file: &Path,
    spec: &str,
    tsconfig: Option<&TsConfig>,
) -> ResolveResult {
    if let Some(tc) = tsconfig {
        if let Some(hit) = resolve_via_tsconfig(project_root, spec, tc) {
            return hit;
        }
    }

    let is_relative = spec.starts_with('.') || spec.starts_with('/');
    if !is_relative {
        // External — canonicalise scoped packages the same way the TS driver does.
        let library = if spec.starts_with('@') {
            let mut parts = spec.splitn(3, '/');
            match (parts.next(), parts.next()) {
                (Some(scope), Some(pkg)) => format!("{scope}/{pkg}"),
                _ => spec.to_owned(),
            }
        } else {
            spec.split('/').next().unwrap_or(spec).to_owned()
        };
        return ResolveResult::External {
            library,
            symbols: Vec::new(),
        };
    }

    let base = from_file.parent().unwrap_or(project_root);
    let joined = normalize_path(&base.join(spec));
    try_ts_suffixes(&joined)
        .or_else(|| try_ts_index(&joined))
        .map_or(ResolveResult::Unresolved, ResolveResult::Internal)
}

/// Normalize a path by resolving `.` and `..` components lexically,
/// without requiring the path to exist on disk (unlike `canonicalize`).
fn normalize_path(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

fn try_ts_suffixes(candidate: &Path) -> Option<PathBuf> {
    // Direct hit (already has an extension like `./foo.ts`).
    if candidate.is_file() {
        return Some(candidate.to_path_buf());
    }
    for ext in ["ts", "tsx", "js", "jsx", "mts", "cts"] {
        let with_ext = candidate.with_extension(ext);
        if with_ext.is_file() {
            return Some(with_ext);
        }
    }
    None
}

fn try_ts_index(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    for ext in ["ts", "tsx", "js", "jsx"] {
        let index = dir.join(format!("index.{ext}"));
        if index.is_file() {
            return Some(index);
        }
    }
    None
}

/// Resolve `spec` against the alias map in `tsconfig`.
///
/// Supports two pattern forms:
/// - **Wildcard**: `"@shared/*"` maps the captured tail into each target with a
///   matching `*` suffix (e.g. `"src/shared/*"`).
/// - **Exact**: `"@config"` maps directly to each target without glob expansion.
///
/// The extension/index ladder from [`try_ts_suffixes`] and [`try_ts_index`] is
/// re-run on the mapped candidate before returning.
fn resolve_via_tsconfig(
    project_root: &Path,
    spec: &str,
    tsconfig: &TsConfig,
) -> Option<ResolveResult> {
    let base = tsconfig.base_url.as_deref().unwrap_or(Path::new("."));

    for (pattern, targets) in &tsconfig.paths {
        if let Some(prefix) = pattern.strip_suffix('*') {
            // Wildcard pattern: "@shared/*" → captures rest after "@shared/".
            if let Some(rest) = spec.strip_prefix(prefix) {
                for target in targets {
                    let Some(t_prefix) = target.strip_suffix('*') else {
                        continue;
                    };
                    let candidate =
                        normalize_path(&project_root.join(base).join(format!("{t_prefix}{rest}")));
                    if let Some(resolved) =
                        try_ts_suffixes(&candidate).or_else(|| try_ts_index(&candidate))
                    {
                        return Some(ResolveResult::Internal(resolved));
                    }
                }
            }
        } else if pattern == spec {
            // Exact pattern: "@config" → maps straight to each target string.
            for target in targets {
                let candidate = normalize_path(&project_root.join(base).join(target));
                if let Some(resolved) =
                    try_ts_suffixes(&candidate).or_else(|| try_ts_index(&candidate))
                {
                    return Some(ResolveResult::Internal(resolved));
                }
            }
        }
    }
    None
}

/// Load `<project_root>/tsconfig.json` and extract `compilerOptions.baseUrl`
/// and `compilerOptions.paths`. Returns `None` when the file is absent.
///
/// `tsconfig.json` allows JSONC-style `//` line comments; this function strips
/// them before parsing with `serde_json`. Block comments (`/* */`) are not
/// supported — they are rare in practice and can be added when needed.
///
/// # Errors
///
/// Returns [`BlastGuardError::Config`] on malformed JSON after comment stripping.
/// Returns [`BlastGuardError::Io`] on filesystem access failures.
#[must_use = "caller must check whether a TsConfig was found"]
pub fn load_tsconfig(project_root: &Path) -> Result<Option<TsConfig>> {
    let path = project_root.join("tsconfig.json");
    if !path.exists() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&path).map_err(|source| BlastGuardError::Io {
        path: path.clone(),
        source,
    })?;
    let stripped = strip_jsonc_comments(&body);
    let v: serde_json::Value = serde_json::from_str(&stripped)
        .map_err(|e| BlastGuardError::Config(format!("tsconfig.json: {e}")))?;

    let co = v.get("compilerOptions");
    let base_url = co
        .and_then(|c| c.get("baseUrl"))
        .and_then(|b| b.as_str())
        .map(PathBuf::from);

    let mut paths_map: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(paths) = co.and_then(|c| c.get("paths")).and_then(|p| p.as_object()) {
        for (k, v) in paths {
            let targets: Vec<String> = v
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(ToOwned::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            paths_map.insert(k.clone(), targets);
        }
    }

    Ok(Some(TsConfig {
        base_url,
        paths: paths_map,
    }))
}

/// Resolve a Python absolute dotted module path to a project file.
///
/// Resolution order for `spec = "utils.auth"`:
/// 1. `<root>/src/utils/auth.py`
/// 2. `<root>/src/utils/auth/__init__.py`
/// 3. `<root>/utils/auth.py`
/// 4. `<root>/utils/auth/__init__.py`
///
/// A bare package name (e.g. `"utils"`) resolves the same way, preferring
/// the `.py` file if it exists, otherwise the `__init__.py`.
///
/// # Notes on the `.` sentinel
///
/// The Python driver stores bare relative imports (`from . import foo`) with a
/// sentinel `library = "."`. When `spec` is `"."` or empty, this function
/// returns [`ResolveResult::Unresolved`] because the dot-count needed to walk
/// up directories is not propagated by the driver yet (Phase 1 limitation).
///
/// The `from_file` parameter is accepted for API consistency with
/// [`resolve_ts`] but is not used in Phase 1 — Python uses absolute module
/// paths rooted at the project, not file-relative specifiers.
#[must_use]
pub fn resolve_py(project_root: &Path, _from_file: &Path, spec: &str) -> ResolveResult {
    if spec.is_empty() || spec == "." {
        // Bare relative import — the driver doesn't propagate dot-count yet.
        // Return Unresolved rather than guessing; Task 10 v2 can revisit.
        return ResolveResult::Unresolved;
    }

    let rel: PathBuf = spec.split('.').collect();
    let candidates = [
        project_root.join("src").join(&rel).with_extension("py"),
        project_root.join("src").join(&rel).join("__init__.py"),
        project_root.join(&rel).with_extension("py"),
        project_root.join(&rel).join("__init__.py"),
    ];
    for c in &candidates {
        if c.is_file() {
            return ResolveResult::Internal(c.clone());
        }
    }

    // Not found on disk — treat first dotted segment as an external library.
    let library = spec.split('.').next().unwrap_or(spec).to_owned();
    ResolveResult::External {
        library,
        symbols: Vec::new(),
    }
}

/// Resolve a Rust `use` path.
///
/// Rules:
/// - `crate::foo::bar` / `self::foo::bar` / `super::foo::bar` → try
///   `<project_root>/src/foo/bar.rs`, then `<project_root>/src/foo/bar/mod.rs`.
///   Returns [`ResolveResult::Unresolved`] when neither exists.
/// - External crate (`tokio::…`, `std::…`) → [`ResolveResult::External`] with
///   the head segment as the library name.
/// - `super::` is treated as `crate::` for Phase 1.3 — walking up from
///   `from_file`'s module is a known Phase 2 refinement.
#[must_use]
pub fn resolve_rs(project_root: &Path, _from_file: &Path, spec: &str) -> ResolveResult {
    let head = spec.split("::").next().unwrap_or("");
    match head {
        "crate" | "self" | "super" => {
            let tail: Vec<&str> = spec.split("::").skip(1).collect();
            if tail.is_empty() {
                return ResolveResult::Unresolved;
            }
            // Walk tail prefixes longest → shortest. `use crate::config::Config`
            // should resolve to `src/config.rs` when no `src/config/Config.rs`
            // exists — the trailing segments are items (types / fns), not
            // modules.
            for len in (1..=tail.len()).rev() {
                let rel: PathBuf = tail[..len].iter().collect();
                let candidates = [
                    project_root.join("src").join(&rel).with_extension("rs"),
                    project_root.join("src").join(&rel).join("mod.rs"),
                ];
                for c in candidates {
                    if c.is_file() {
                        return ResolveResult::Internal(c);
                    }
                }
            }
            ResolveResult::Unresolved
        }
        "" => ResolveResult::Unresolved,
        _ => ResolveResult::External {
            library: head.to_owned(),
            symbols: Vec::new(),
        },
    }
}

/// Walk every `EdgeKind::Imports` edge with [`Confidence::Unresolved`] and
/// attempt to rewrite `to.file` to a real on-disk path, upgrading the edge to
/// [`Confidence::Certain`] on success. Edges whose `from` file is not a
/// supported language — or whose spec cannot be resolved — are left untouched.
///
/// The spec text for each edge lives in `edge.to.file` (the parsers stash the
/// raw `use crate::foo` / `"./utils"` string there until resolution happens).
/// After resolution, `to.file` points at the resolved module file and
/// `importers_of(file)` / cross-file callers hints become usable.
///
/// # Why this lives in the indexer pipeline
/// Parsers run per-file and do not know the project root, tsconfig, or which
/// modules exist — resolving in the parser would require threading all three
/// through every worker. Doing it here is O(edges) and runs once per
/// cold/warm index.
pub fn resolve_imports(graph: &mut CodeGraph, project_root: &Path) {
    let tsconfig = load_tsconfig(project_root).ok().flatten();

    // Phase 1: collect rewrites so we don't mutate while iterating.
    let mut rewrites: Vec<(SymbolId, usize, PathBuf)> = Vec::new();
    for (from_id, edges) in &graph.forward_edges {
        for (idx, edge) in edges.iter().enumerate() {
            if edge.kind != EdgeKind::Imports || edge.confidence != Confidence::Unresolved {
                continue;
            }
            let spec = edge.to.file.to_string_lossy();
            let ext = from_id
                .file
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let result = match ext {
                "rs" => resolve_rs(project_root, &from_id.file, &spec),
                "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => {
                    resolve_ts(project_root, &from_id.file, &spec, tsconfig.as_ref())
                }
                "py" | "pyi" => resolve_py(project_root, &from_id.file, &spec),
                _ => continue,
            };
            if let ResolveResult::Internal(new_path) = result {
                rewrites.push((from_id.clone(), idx, new_path));
            }
        }
    }

    // Phase 2: apply each rewrite — update forward_edges in place, move the
    // edge in reverse_edges from old_to to new_to, fix centrality.
    for (from_id, idx, new_path) in rewrites {
        let Some(edges) = graph.forward_edges.get_mut(&from_id) else {
            continue;
        };
        let Some(edge) = edges.get_mut(idx) else {
            continue;
        };
        let old_to = edge.to.clone();
        if old_to.file == new_path {
            continue;
        }
        let mut new_to = old_to.clone();
        new_to.file = new_path;
        edge.to = new_to.clone();
        edge.confidence = Confidence::Certain;
        let updated_edge = edge.clone();

        if let Some(rev_list) = graph.reverse_edges.get_mut(&old_to) {
            rev_list.retain(|e| !(e.from == from_id && e.kind == EdgeKind::Imports));
            if rev_list.is_empty() {
                graph.reverse_edges.remove(&old_to);
            }
        }
        graph
            .reverse_edges
            .entry(new_to.clone())
            .or_default()
            .push(updated_edge);

        if let Some(c) = graph.centrality.get_mut(&old_to) {
            *c = c.saturating_sub(1);
        }
        *graph.centrality.entry(new_to).or_insert(0) += 1;
    }
}

/// Strip `//` line comments from a JSONC string.
///
/// Only line comments are handled. Block comments are rare in tsconfig files
/// and not supported here — add when benchmark projects require it.
fn strip_jsonc_comments(src: &str) -> String {
    src.lines()
        .map(|line| match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        for (path, body) in files {
            let full = tmp.path().join(path);
            std::fs::create_dir_all(full.parent().expect("parent dir")).expect("mkdir");
            std::fs::write(&full, body).expect("write");
        }
        tmp
    }

    #[test]
    fn resolves_relative_ts_file() {
        let tmp = tempdir_with(&[("src/handler.ts", ""), ("src/utils/auth.ts", "")]);
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "./utils/auth", None);
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/utils/auth.ts"))
        );
    }

    #[test]
    fn resolves_tsx_extension() {
        let tmp = tempdir_with(&[("src/app.ts", ""), ("src/Button.tsx", "")]);
        let from = tmp.path().join("src/app.ts");
        let r = resolve_ts(tmp.path(), &from, "./Button", None);
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/Button.tsx"))
        );
    }

    #[test]
    fn resolves_index_file_in_directory() {
        let tmp = tempdir_with(&[("src/handler.ts", ""), ("src/utils/index.ts", "")]);
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "./utils", None);
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/utils/index.ts"))
        );
    }

    #[test]
    fn resolves_parent_directory_import() {
        let tmp = tempdir_with(&[("src/nested/deep.ts", ""), ("src/shared.ts", "")]);
        let from = tmp.path().join("src/nested/deep.ts");
        let r = resolve_ts(tmp.path(), &from, "../shared", None);
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/shared.ts")));
    }

    #[test]
    fn bare_specifier_is_external() {
        let tmp = tempdir_with(&[("src/a.ts", "")]);
        let from = tmp.path().join("src/a.ts");
        let r = resolve_ts(tmp.path(), &from, "lodash", None);
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "lodash"),
            _ => panic!("expected External, got {r:?}"),
        }
    }

    #[test]
    fn scoped_bare_specifier_is_external_with_full_scope() {
        let tmp = tempdir_with(&[("src/a.ts", "")]);
        let from = tmp.path().join("src/a.ts");
        let r = resolve_ts(tmp.path(), &from, "@scope/pkg", None);
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "@scope/pkg"),
            _ => panic!("expected External, got {r:?}"),
        }
    }

    #[test]
    fn unresolved_when_no_match() {
        let tmp = tempdir_with(&[("src/a.ts", "")]);
        let from = tmp.path().join("src/a.ts");
        let r = resolve_ts(tmp.path(), &from, "./missing", None);
        assert_eq!(r, ResolveResult::Unresolved);
    }

    #[test]
    fn resolves_via_tsconfig_path_alias() {
        let tmp = tempdir_with(&[("src/handler.ts", ""), ("src/shared/auth.ts", "")]);
        let tc = TsConfig {
            base_url: Some(PathBuf::from(".")),
            paths: HashMap::from([("@shared/*".to_string(), vec!["src/shared/*".to_string()])]),
        };
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "@shared/auth", Some(&tc));
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/shared/auth.ts"))
        );
    }

    #[test]
    fn resolves_exact_tsconfig_alias_no_wildcard() {
        let tmp = tempdir_with(&[("src/handler.ts", ""), ("src/config.ts", "")]);
        let tc = TsConfig {
            base_url: Some(PathBuf::from(".")),
            paths: HashMap::from([("@config".to_string(), vec!["src/config".to_string()])]),
        };
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "@config", Some(&tc));
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/config.ts")));
    }

    #[test]
    fn loads_tsconfig_from_disk() {
        let tmp = tempdir_with(&[(
            "tsconfig.json",
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": { "@shared/*": ["src/shared/*"] }
                }
            }"#,
        )]);
        let tc = load_tsconfig(tmp.path())
            .expect("load ok")
            .expect("present");
        assert!(tc.paths.contains_key("@shared/*"));
        assert_eq!(tc.base_url, Some(PathBuf::from(".")));
    }

    #[test]
    fn loads_tsconfig_with_jsonc_comments() {
        let tmp = tempdir_with(&[(
            "tsconfig.json",
            r#"{
                // Top-level comment
                "compilerOptions": {
                    "baseUrl": ".",
                    // inline comment
                    "paths": { "@util/*": ["src/util/*"] }
                }
            }"#,
        )]);
        let tc = load_tsconfig(tmp.path())
            .expect("load ok")
            .expect("present");
        assert!(tc.paths.contains_key("@util/*"));
    }

    #[test]
    fn no_tsconfig_returns_ok_none() {
        let tmp = tempdir_with(&[("src/a.ts", "")]);
        let tc = load_tsconfig(tmp.path()).expect("no-err on missing");
        assert!(tc.is_none());
    }

    #[test]
    fn malformed_tsconfig_returns_err() {
        let tmp = tempdir_with(&[("tsconfig.json", "not valid json {")]);
        let err = load_tsconfig(tmp.path()).expect_err("malformed json must error");
        let s = format!("{err}");
        assert!(
            s.to_lowercase().contains("tsconfig") || s.to_lowercase().contains("config"),
            "error should mention tsconfig; got {s}"
        );
    }

    // ── Python resolver tests ─────────────────────────────────────────────────

    #[test]
    fn resolves_python_dotted_module_under_src() {
        let tmp = tempdir_with(&[
            ("src/handler.py", ""),
            ("src/utils/auth.py", ""),
            ("src/utils/__init__.py", ""),
        ]);
        let from = tmp.path().join("src/handler.py");
        let r = resolve_py(tmp.path(), &from, "utils.auth");
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/utils/auth.py"))
        );
    }

    #[test]
    fn resolves_python_package_init() {
        let tmp = tempdir_with(&[("src/handler.py", ""), ("src/utils/__init__.py", "")]);
        let from = tmp.path().join("src/handler.py");
        let r = resolve_py(tmp.path(), &from, "utils");
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/utils/__init__.py"))
        );
    }

    #[test]
    fn resolves_python_module_without_src_prefix() {
        let tmp = tempdir_with(&[("handler.py", ""), ("utils/auth.py", "")]);
        let from = tmp.path().join("handler.py");
        let r = resolve_py(tmp.path(), &from, "utils.auth");
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("utils/auth.py")));
    }

    #[test]
    fn unresolved_python_falls_back_to_library_as_external() {
        let tmp = tempdir_with(&[("src/a.py", "")]);
        let from = tmp.path().join("src/a.py");
        let r = resolve_py(tmp.path(), &from, "numpy");
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "numpy"),
            _ => panic!("expected External, got {r:?}"),
        }
    }

    #[test]
    fn python_dotted_external_keeps_first_segment() {
        let tmp = tempdir_with(&[("src/a.py", "")]);
        let from = tmp.path().join("src/a.py");
        let r = resolve_py(tmp.path(), &from, "numpy.linalg");
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "numpy"),
            _ => panic!("expected External, got {r:?}"),
        }
    }

    #[test]
    fn python_dot_sentinel_is_unresolved() {
        let tmp = tempdir_with(&[("src/a.py", "")]);
        let from = tmp.path().join("src/a.py");
        let r = resolve_py(tmp.path(), &from, ".");
        assert_eq!(r, ResolveResult::Unresolved);
    }

    // ── Rust resolver tests ───────────────────────────────────────────────────

    #[test]
    fn resolves_rust_crate_path_to_file() {
        let tmp = tempdir_with(&[
            ("src/main.rs", ""),
            ("src/utils/auth.rs", ""),
            ("src/utils/mod.rs", ""),
        ]);
        let from = tmp.path().join("src/main.rs");
        let r = resolve_rs(tmp.path(), &from, "crate::utils::auth");
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/utils/auth.rs"))
        );
    }

    #[test]
    fn resolves_rust_mod_rs_for_package() {
        let tmp = tempdir_with(&[("src/main.rs", ""), ("src/utils/mod.rs", "")]);
        let from = tmp.path().join("src/main.rs");
        let r = resolve_rs(tmp.path(), &from, "crate::utils");
        assert_eq!(
            r,
            ResolveResult::Internal(tmp.path().join("src/utils/mod.rs"))
        );
    }

    #[test]
    fn resolves_rust_with_self_prefix() {
        let tmp = tempdir_with(&[("src/a.rs", ""), ("src/helper.rs", "")]);
        let from = tmp.path().join("src/a.rs");
        let r = resolve_rs(tmp.path(), &from, "self::helper");
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/helper.rs")));
    }

    #[test]
    fn external_crate_path_is_external() {
        let tmp = tempdir_with(&[("src/main.rs", "")]);
        let from = tmp.path().join("src/main.rs");
        let r = resolve_rs(tmp.path(), &from, "tokio::sync::Mutex");
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "tokio"),
            _ => panic!("expected External, got {r:?}"),
        }
    }

    #[test]
    fn std_path_is_external() {
        let tmp = tempdir_with(&[("src/main.rs", "")]);
        let from = tmp.path().join("src/main.rs");
        let r = resolve_rs(tmp.path(), &from, "std::collections::HashMap");
        match r {
            ResolveResult::External { library, .. } => assert_eq!(library, "std"),
            _ => panic!("expected External, got {r:?}"),
        }
    }

    #[test]
    fn unresolved_rust_crate_path_when_file_absent() {
        let tmp = tempdir_with(&[("src/main.rs", "")]);
        let from = tmp.path().join("src/main.rs");
        let r = resolve_rs(tmp.path(), &from, "crate::missing::mod");
        assert_eq!(r, ResolveResult::Unresolved);
    }

    #[test]
    fn resolves_rust_type_import_by_walking_back_to_module() {
        // `use crate::config::Config` → Config is a type, not a module.
        // Expect resolution to fall back to src/config.rs.
        let tmp = tempdir_with(&[("src/main.rs", ""), ("src/config.rs", "")]);
        let from = tmp.path().join("src/main.rs");
        let r = resolve_rs(tmp.path(), &from, "crate::config::Config");
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/config.rs")));
    }

    #[test]
    fn bare_crate_without_path_is_unresolved() {
        let tmp = tempdir_with(&[("src/main.rs", "")]);
        let from = tmp.path().join("src/main.rs");
        // `use crate;` has no tail segment — ambiguous.
        let r = resolve_rs(tmp.path(), &from, "crate");
        assert_eq!(r, ResolveResult::Unresolved);
    }

    // ── resolve_imports (graph-wide post-parse pass) ──────────────────────────

    #[test]
    fn resolve_imports_upgrades_rust_crate_edge_to_certain() {
        use crate::graph::types::{Edge, EdgeKind, SymbolId, SymbolKind};

        let tmp = tempdir_with(&[("src/main.rs", ""), ("src/utils.rs", "")]);
        let main_rs = tmp.path().join("src/main.rs");
        let utils_rs = tmp.path().join("src/utils.rs");

        let mut graph = CodeGraph::new();
        // Mirror what rust.rs emits: to.file is the raw "crate::utils" text.
        graph.insert_edge(Edge {
            from: SymbolId {
                file: main_rs.clone(),
                name: "main".to_owned(),
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: PathBuf::from("crate::utils"),
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Unresolved,
        });

        resolve_imports(&mut graph, tmp.path());

        let edges = graph.forward_edges.values().flatten().collect::<Vec<_>>();
        assert_eq!(edges.len(), 1, "expected exactly one edge");
        assert_eq!(edges[0].to.file, utils_rs, "to.file should be resolved");
        assert_eq!(
            edges[0].confidence,
            Confidence::Certain,
            "edge should be upgraded to Certain"
        );

        // Reverse index must be keyed by the NEW to (resolved file).
        let resolved_key = SymbolId {
            file: utils_rs.clone(),
            name: String::new(),
            kind: SymbolKind::Module,
        };
        assert!(
            graph.reverse_edges.contains_key(&resolved_key),
            "reverse_edges should be rekeyed to the resolved file"
        );
        // Stale key should be gone.
        let stale_key = SymbolId {
            file: PathBuf::from("crate::utils"),
            name: String::new(),
            kind: SymbolKind::Module,
        };
        assert!(
            !graph.reverse_edges.contains_key(&stale_key),
            "stale reverse_edges entry should be removed"
        );
    }

    #[test]
    fn resolve_imports_leaves_unresolvable_edges_alone() {
        use crate::graph::types::{Edge, EdgeKind, SymbolId, SymbolKind};

        let tmp = tempdir_with(&[("src/main.rs", "")]);
        let main_rs = tmp.path().join("src/main.rs");

        let mut graph = CodeGraph::new();
        graph.insert_edge(Edge {
            from: SymbolId {
                file: main_rs.clone(),
                name: "main".to_owned(),
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: PathBuf::from("crate::does_not_exist"),
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Unresolved,
        });

        resolve_imports(&mut graph, tmp.path());

        let edges = graph.forward_edges.values().flatten().collect::<Vec<_>>();
        assert_eq!(edges.len(), 1);
        assert_eq!(
            edges[0].to.file,
            PathBuf::from("crate::does_not_exist"),
            "unresolvable spec should remain untouched"
        );
        assert_eq!(edges[0].confidence, Confidence::Unresolved);
    }

    #[test]
    fn resolve_imports_upgrades_python_dotted_edge() {
        use crate::graph::types::{Edge, EdgeKind, SymbolId, SymbolKind};

        let tmp = tempdir_with(&[("src/handler.py", ""), ("src/utils/auth.py", "")]);
        let handler = tmp.path().join("src/handler.py");
        let auth = tmp.path().join("src/utils/auth.py");

        let mut graph = CodeGraph::new();
        graph.insert_edge(Edge {
            from: SymbolId {
                file: handler.clone(),
                name: "handler".to_owned(),
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: PathBuf::from("utils.auth"),
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Unresolved,
        });

        resolve_imports(&mut graph, tmp.path());

        let edges = graph
            .forward_edges
            .values()
            .flatten()
            .collect::<Vec<_>>();
        assert_eq!(edges[0].to.file, auth);
        assert_eq!(edges[0].confidence, Confidence::Certain);
    }

    #[test]
    fn resolve_imports_upgrades_ts_relative_edge() {
        use crate::graph::types::{Edge, EdgeKind, SymbolId, SymbolKind};

        let tmp = tempdir_with(&[("src/handler.ts", ""), ("src/utils/auth.ts", "")]);
        let handler = tmp.path().join("src/handler.ts");
        let auth = tmp.path().join("src/utils/auth.ts");

        let mut graph = CodeGraph::new();
        graph.insert_edge(Edge {
            from: SymbolId {
                file: handler.clone(),
                name: "handler".to_owned(),
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: PathBuf::from("./utils/auth"),
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Unresolved,
        });

        resolve_imports(&mut graph, tmp.path());

        let edges = graph.forward_edges.values().flatten().collect::<Vec<_>>();
        assert_eq!(edges[0].to.file, auth);
        assert_eq!(edges[0].confidence, Confidence::Certain);
    }
}
