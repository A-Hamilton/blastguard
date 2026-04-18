//! Import path resolution — SPEC §6.
//!
//! TS/JS follow the extension/index ladder, honouring `tsconfig.json`
//! `compilerOptions.paths` and `baseUrl` (Task 9). Python resolves via the
//! package tree (Task 10). Rust uses the `mod` hierarchy rooted at `src/`
//! (Task 11).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{BlastGuardError, Result};

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
    let base = tsconfig
        .base_url
        .as_deref()
        .unwrap_or(Path::new("."));

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
        let tmp = tempdir_with(&[
            ("src/handler.ts", ""),
            ("src/utils/auth.ts", ""),
        ]);
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "./utils/auth", None);
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/auth.ts")));
    }

    #[test]
    fn resolves_tsx_extension() {
        let tmp = tempdir_with(&[
            ("src/app.ts", ""),
            ("src/Button.tsx", ""),
        ]);
        let from = tmp.path().join("src/app.ts");
        let r = resolve_ts(tmp.path(), &from, "./Button", None);
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/Button.tsx")));
    }

    #[test]
    fn resolves_index_file_in_directory() {
        let tmp = tempdir_with(&[
            ("src/handler.ts", ""),
            ("src/utils/index.ts", ""),
        ]);
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "./utils", None);
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/index.ts")));
    }

    #[test]
    fn resolves_parent_directory_import() {
        let tmp = tempdir_with(&[
            ("src/nested/deep.ts", ""),
            ("src/shared.ts", ""),
        ]);
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
        let tmp = tempdir_with(&[
            ("src/handler.ts", ""),
            ("src/shared/auth.ts", ""),
        ]);
        let tc = TsConfig {
            base_url: Some(PathBuf::from(".")),
            paths: HashMap::from([(
                "@shared/*".to_string(),
                vec!["src/shared/*".to_string()],
            )]),
        };
        let from = tmp.path().join("src/handler.ts");
        let r = resolve_ts(tmp.path(), &from, "@shared/auth", Some(&tc));
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/shared/auth.ts")));
    }

    #[test]
    fn resolves_exact_tsconfig_alias_no_wildcard() {
        let tmp = tempdir_with(&[
            ("src/handler.ts", ""),
            ("src/config.ts", ""),
        ]);
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
        let tmp = tempdir_with(&[
            ("tsconfig.json", r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": { "@shared/*": ["src/shared/*"] }
                }
            }"#),
        ]);
        let tc = load_tsconfig(tmp.path()).expect("load ok").expect("present");
        assert!(tc.paths.contains_key("@shared/*"));
        assert_eq!(tc.base_url, Some(PathBuf::from(".")));
    }

    #[test]
    fn loads_tsconfig_with_jsonc_comments() {
        let tmp = tempdir_with(&[
            ("tsconfig.json", r#"{
                // Top-level comment
                "compilerOptions": {
                    "baseUrl": ".",
                    // inline comment
                    "paths": { "@util/*": ["src/util/*"] }
                }
            }"#),
        ]);
        let tc = load_tsconfig(tmp.path()).expect("load ok").expect("present");
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
        let tmp = tempdir_with(&[
            ("tsconfig.json", "not valid json {"),
        ]);
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
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/auth.py")));
    }

    #[test]
    fn resolves_python_package_init() {
        let tmp = tempdir_with(&[
            ("src/handler.py", ""),
            ("src/utils/__init__.py", ""),
        ]);
        let from = tmp.path().join("src/handler.py");
        let r = resolve_py(tmp.path(), &from, "utils");
        assert_eq!(r, ResolveResult::Internal(tmp.path().join("src/utils/__init__.py")));
    }

    #[test]
    fn resolves_python_module_without_src_prefix() {
        let tmp = tempdir_with(&[
            ("handler.py", ""),
            ("utils/auth.py", ""),
        ]);
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
}
