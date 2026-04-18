//! Import path resolution — SPEC §6.
//!
//! TS/JS follow the extension/index ladder, honouring `tsconfig.json`
//! `compilerOptions.paths` and `baseUrl` (Task 9). Python resolves via the
//! package tree (Task 10). Rust uses the `mod` hierarchy rooted at `src/`
//! (Task 11).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

/// Task 9 fills this in with baseUrl + paths handling. For Task 8 it is a stub.
fn resolve_via_tsconfig(
    _project_root: &Path,
    _spec: &str,
    _tsconfig: &TsConfig,
) -> Option<ResolveResult> {
    None
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
}
