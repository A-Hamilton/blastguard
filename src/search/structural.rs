//! Graph-backed search backends — SPEC §3.1.
//!
//! Each public function resolves a [`super::query::QueryKind`] arm against the
//! [`CodeGraph`] and renders hits via [`super::hit::SearchHit::structural`].

use crate::graph::ops::{callees, callers, find_by_name, shortest_path};
use crate::graph::types::{CodeGraph, Confidence, EdgeKind, Symbol, SymbolId};
use crate::search::hit::{sort_by_centrality, SearchHit};

/// `find X` / `where is X` — centrality-ranked name lookup with fuzzy fallback.
/// Returns at most `max_hits` results.
#[must_use]
pub fn find(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut ids: Vec<&SymbolId> = find_by_name(graph, name);
    if ids.is_empty() {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{name}' found; try `grep {name}` for text search"
        ))];
    }
    sort_by_centrality(graph, &mut ids);
    let mut hits: Vec<SearchHit> = ids
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect();

    // Prepend a count header so the agent has immediate completeness
    // confidence, reducing distrust-driven re-queries (same pattern as
    // callers_of count header which saved -31.88% on callers-apply-edit).
    let symbol_count = hits.len();
    let file_count = {
        let mut files: std::collections::BTreeSet<&std::path::Path> =
            std::collections::BTreeSet::new();
        for h in &hits {
            if !h.file.as_os_str().is_empty() {
                files.insert(h.file.as_path());
            }
        }
        files.len()
    };
    let file_names: Vec<String> = {
        let mut names: Vec<String> = hits
            .iter()
            .filter(|h| !h.file.as_os_str().is_empty())
            .map(|h| {
                h.file
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
            .collect();
        names.sort();
        names.dedup();
        names
    };
    let count_hint = if symbol_count > 0 {
        let files_part = if file_count == 1 {
            format!(" in {}", file_names.first().map_or("?", |s| s.as_str()))
        } else {
            let listed: Vec<&str> = file_names
                .iter()
                .take(3)
                .map(std::string::String::as_str)
                .collect();
            let more = file_count.saturating_sub(3);
            if more > 0 {
                format!(
                    " across {file_count} files (e.g. {}, +{more} more)",
                    listed.join(", "),
                )
            } else {
                format!(" across {file_count} files ({})", listed.join(", "))
            }
        };
        format!(
            "=== {symbol_count} symbol{} matching '{name}'{files_part} ===",
            if symbol_count == 1 { "" } else { "s" },
        )
    } else {
        format!("=== 0 symbols matching '{name}' ===")
    };
    hits.insert(0, SearchHit::empty_hint(&count_hint));
    hits
}

/// Caller lookup by pre-resolved [`SymbolId`]. Used by Plan 3's
/// `apply_change` bundled context so the orchestrator can feed the exact
/// edited symbol rather than re-resolving by name (which risks picking a
/// different same-named symbol).
#[must_use]
pub fn callers_of_id(graph: &CodeGraph, id: &SymbolId, max_hits: usize) -> Vec<SearchHit> {
    let mut caller_ids: Vec<&SymbolId> = callers(graph, id);
    // Fallback to (file, name) match to catch Phase 1.2 driver placeholders
    // where call edges have to.kind=Function regardless of the callee's
    // actual kind. Same pattern as impact::find_callers_by_name.
    if caller_ids.is_empty() {
        caller_ids = graph
            .reverse_edges
            .iter()
            .filter(|(to_id, _)| to_id.file == id.file && to_id.name == id.name)
            .flat_map(|(_, edges)| edges.iter().map(|e| &e.from))
            .collect();
    }
    sort_by_centrality(graph, &mut caller_ids);
    let mut hits: Vec<SearchHit> = caller_ids
        .into_iter()
        .take(max_hits)
        .filter_map(|cid| graph.symbols.get(cid))
        .map(|s| SearchHit::structural(s).without_return_type())
        .collect();

    // Prepend a count header for completeness confidence.
    let caller_count = hits.len();
    let file_count = {
        let mut files: std::collections::BTreeSet<&std::path::Path> =
            std::collections::BTreeSet::new();
        for h in &hits {
            if !h.file.as_os_str().is_empty() {
                files.insert(h.file.as_path());
            }
        }
        files.len()
    };
    let count_hint = if caller_count > 0 {
        format!(
            "=== {caller_count} caller{} of {} in {file_count} file{} ===",
            if caller_count == 1 { "" } else { "s" },
            id.name,
            if file_count == 1 { "" } else { "s" },
        )
    } else {
        format!("=== 0 callers of {} ===", id.name)
    };
    hits.insert(0, SearchHit::empty_hint(&count_hint));
    hits
}

/// `callers of X` / `what calls X` — reverse BFS (1 hop) with inline signatures.
///
/// Resolves `name` to the most-central exact-match symbol, then returns its
/// direct callers sorted by their own centrality descending, capped at
/// `max_hits`.
///
/// `project_root` is only used to render the cross-file importer hint's
/// embedded target path as relative — pass the indexed project root.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn callers_of(
    graph: &CodeGraph,
    name: &str,
    max_hits: usize,
    project_root: &std::path::Path,
    with_context: bool,
) -> Vec<SearchHit> {
    let mut targets = find_by_name(graph, name);
    if targets.is_empty() {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{name}' found; try `find {name}` for fuzzy matches or grep across files"
        ))];
    }
    sort_by_centrality(graph, &mut targets);
    let Some(&target_id) = targets.first() else {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{name}' found; try `find {name}` for fuzzy matches or grep across files"
        ))];
    };
    let mut caller_ids: Vec<&SymbolId> = callers(graph, target_id);
    sort_by_centrality(graph, &mut caller_ids);
    let mut hits: Vec<SearchHit> = caller_ids
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(|s| SearchHit::structural(s).without_return_type())
        .collect();

    if with_context {
        // Per-hit context uses the CALL-SITE line from the graph edge,
        // not the caller's definition line (which is hit.line). The
        // caller's `fn foo(...)` declaration is on hit.line; the actual
        // call to the target lives further down inside the body. Look
        // up the edge in reverse_edges[target_id] where edge.from ==
        // caller_id to get the right line.
        let edge_line_for_caller: std::collections::HashMap<&SymbolId, u32> = graph
            .reverse_edges
            .get(target_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Calls)
                    .map(|e| (&e.from, e.line))
                    .collect()
            })
            .unwrap_or_default();
        for hit in &mut hits {
            if hit.is_hint() {
                continue;
            }
            // Find the caller symbol for this hit's file + line_start
            // pairing, then look up its edge line.
            let caller_id = graph.symbols.iter().find_map(|(id, sym)| {
                (sym.id.file == hit.file && sym.line_start == hit.line).then_some(id)
            });
            let call_site_line = caller_id
                .and_then(|id| edge_line_for_caller.get(id).copied())
                .unwrap_or(hit.line); // fallback: caller def line
            hit.context =
                crate::search::context_extract::enclosing_statement(&hit.file, call_site_line);
            // Prepend the callee's compact signature as a comment prefix so
            // the agent sees parameter types inline with each caller hit,
            // eliminating the need for a separate `find` call to understand
            // what arguments to pass in apply_edit.
            if let Some(target_sym) = graph.symbols.get(target_id) {
                if let Some(ctx) = &mut hit.context {
                    let callee_prefix = format!("// callee: {}", target_sym.signature);
                    *ctx = format!("{callee_prefix}\n{ctx}");
                    // Extract the callee name from the signature (e.g. "fn foo(x: i32)" -> "foo")
                    // and look up the actual argument expressions at the call site.
                    let callee_name = target_sym
                        .signature
                        .split_once(' ')
                        .and_then(|(_prefix, rest)| rest.split_once('('))
                        .map(|(name, _)| name.trim());
                    if let Some(name) = callee_name {
                        if let Some(args) = crate::search::context_extract::extract_call_args(
                            &hit.file,
                            call_site_line,
                            name,
                        ) {
                            let args_prefix = format!("// args: {args}");
                            *ctx = format!("{args_prefix}\n{ctx}");
                        }
                    }
                }
            }
        }
    }

    // Cross-file importer hint (Phase-1.5 fallback): when resolve_calls
    // couldn't pin a function-level caller (ambiguous match or non-function
    // reference like a type), at least tell the agent which files import
    // the target's module so they can grep directly. Skip files that
    // already produced a first-class caller hit above — those are
    // redundant noise.
    let target_file = &target_id.file;
    let target_rel = target_file
        .strip_prefix(project_root)
        .unwrap_or(target_file);
    let files_with_function_callers: std::collections::HashSet<std::path::PathBuf> =
        hits.iter().map(|h| h.file.clone()).collect();
    let importer_hits = importers_of(graph, target_file, project_root);
    let mut seen_importers: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for hit in &importer_hits {
        if hit.file == *target_file {
            continue; // self-import impossible in practice; defensive
        }
        if files_with_function_callers.contains(&hit.file) {
            continue; // we already surfaced the specific function; hint is noise
        }
        if !seen_importers.insert(hit.file.clone()) {
            continue; // one hint per importer file, not per import statement
        }
        hits.push(SearchHit {
            file: hit.file.clone(),
            line: hit.line,
            signature: Some(format!(
                "cross-file importer — file imports `{}`; grep `{name}` here for call sites",
                target_rel.display()
            )),
            snippet: None,
            context: None,
        });
    }

    if hits.is_empty() {
        return vec![SearchHit::empty_hint(
            "no same-file callers and no cross-file importers in Phase 1 graph; use grep",
        )];
    }

    // Prepend a count header so the agent has immediate completeness
    // confidence, reducing distrust-driven re-queries (analogous to the
    // outline summary header which saved -55.2% on outline-tree-sitter-rust).
    let caller_count = hits.iter().filter(|h| !h.is_hint()).count();
    let file_count = {
        let mut files: std::collections::BTreeSet<&std::path::Path> =
            std::collections::BTreeSet::new();
        for h in &hits {
            if !h.is_hint() && !h.file.as_os_str().is_empty() {
                files.insert(h.file.as_path());
            }
        }
        files.len()
    };
    let count_hint = if caller_count > 0 {
        format!(
            "=== {caller_count} caller{} of {name} in {file_count} file{} ===",
            if caller_count == 1 { "" } else { "s" },
            if file_count == 1 { "" } else { "s" },
        )
    } else {
        format!("=== 0 callers of {name} (only cross-file importers) ===")
    };
    hits.insert(0, SearchHit::empty_hint(&count_hint));
    hits
}

/// `callees of X` / `what does X call` — forward edges with inline signatures.
#[must_use]
pub fn callees_of(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut sources = find_by_name(graph, name);
    if sources.is_empty() {
        return Vec::new();
    }
    sort_by_centrality(graph, &mut sources);
    let Some(&source_id) = sources.first() else {
        return Vec::new();
    };
    let mut callee_ids: Vec<&SymbolId> = callees(graph, source_id);
    sort_by_centrality(graph, &mut callee_ids);
    callee_ids
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}

/// `chain from X to Y` — BFS shortest path across forward call edges.
///
/// Two modes:
///
/// - **Symbol mode** (`Y` is a bare name): returns the shortest `Vec` of
///   structural hits from the `from` symbol to the `to` symbol. If either
///   name doesn't resolve, or no path exists, returns a hint-shaped hit
///   with guidance.
/// - **Path mode** (`Y` is a file path — contains `/`, `\`, or ends in a
///   known source extension): returns the shortest chain from `from` into
///   any symbol whose file matches `Y`, followed by structural hits for
///   every other symbol in the target file that is a direct Calls-successor
///   of any node on the chain. Agents use this to answer "which function
///   in FILE does X reach?" in a single query.
///
/// Empty `Vec` is reserved for "neither endpoint exists" so the dispatcher
/// can distinguish the cases.
#[must_use]
pub fn chain_from_to(graph: &CodeGraph, from_name: &str, to_name: &str) -> Vec<SearchHit> {
    let from_ids = find_by_name(graph, from_name);
    let Some(&from_id) = from_ids.first() else {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{from_name}' found; try `find {from_name}` for fuzzy matches"
        ))];
    };

    if is_path_like(to_name) {
        return chain_to_file_path(graph, from_id, to_name);
    }

    let to_ids = find_by_name(graph, to_name);
    let Some(&to_id) = to_ids.first() else {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{to_name}' found; try `find {to_name}` for fuzzy matches"
        ))];
    };
    if let Some(path) = shortest_path(graph, from_id, to_id) {
        return path
            .iter()
            .filter_map(|id| graph.symbols.get(id))
            .map(SearchHit::structural)
            .collect();
    }

    // No graph path — return the two endpoints plus a hint so the agent
    // has somewhere to go next instead of a dead-end empty response.
    let mut hits: Vec<SearchHit> = Vec::new();
    if let Some(sym) = graph.symbols.get(from_id) {
        hits.push(SearchHit::structural(sym));
    }
    if let Some(sym) = graph.symbols.get(to_id) {
        hits.push(SearchHit::structural(sym));
    }
    hits.push(SearchHit::empty_hint(&format!(
        "no call-graph path from {from_name} to {to_name} — Phase 1 doesn't follow re-export chains (`pub use`) or dynamic dispatch. \
         Try `imports of <from-file>` and `callers of {to_name}` to bridge manually, or grep for intermediate call sites."
    )));
    hits
}

/// Path-mode of `chain_from_to`: BFS from `from_id` to the first symbol in
/// the target file, plus sibling Calls-successors in the same file.
///
/// Caps the candidate list at [`CHAIN_FILE_CANDIDATE_CAP`] to keep the
/// response bounded on large target files.
fn chain_to_file_path(graph: &CodeGraph, from_id: &SymbolId, to_path: &str) -> Vec<SearchHit> {
    use std::path::Path;

    let target = Path::new(to_path);

    // File not indexed at all → useful hint so the agent doesn't retry blindly.
    let any_symbol_in_file = graph.symbols.keys().any(|id| id.file.ends_with(target));
    if !any_symbol_in_file {
        return vec![SearchHit::empty_hint(&format!(
            "no symbols indexed in {to_path}; try `outline of {to_path}` or check the path spelling"
        ))];
    }

    let Some(path) = crate::graph::ops::shortest_path_to_predicate(graph, from_id, |id| {
        id.file.ends_with(target)
    }) else {
        // Indexed but unreachable via forward call edges.
        let mut hits: Vec<SearchHit> = Vec::new();
        if let Some(sym) = graph.symbols.get(from_id) {
            hits.push(SearchHit::structural(sym));
        }
        hits.push(SearchHit::empty_hint(&format!(
            "no call-graph path from {from_name} to any symbol in {to_path}; try `callees of {from_name}` then filter by file",
            from_name = from_id.name,
        )));
        return hits;
    };

    let mut hits: Vec<SearchHit> = path
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect();

    // Sibling candidates: symbols in the target file that are direct
    // Calls-successors of any node on the chain, excluding chain nodes.
    let chain_ids: std::collections::HashSet<&SymbolId> = path.iter().collect();
    let mut seen: std::collections::HashSet<&SymbolId> = std::collections::HashSet::new();
    let mut candidates: Vec<&Symbol> = Vec::new();
    for node in &path {
        let Some(edges) = graph.forward_edges.get(node) else {
            continue;
        };
        for e in edges {
            if e.kind != EdgeKind::Calls {
                continue;
            }
            if !e.to.file.ends_with(target) {
                continue;
            }
            if chain_ids.contains(&e.to) {
                continue;
            }
            if !seen.insert(&e.to) {
                continue;
            }
            if let Some(sym) = graph.symbols.get(&e.to) {
                candidates.push(sym);
            }
        }
    }
    // Centrality-sorted, highest first.
    candidates
        .sort_by_key(|s| std::cmp::Reverse(graph.centrality.get(&s.id).copied().unwrap_or(0)));
    let overflow = candidates.len().saturating_sub(CHAIN_FILE_CANDIDATE_CAP);
    for sym in candidates.into_iter().take(CHAIN_FILE_CANDIDATE_CAP) {
        hits.push(SearchHit::structural(sym));
    }
    if overflow > 0 {
        hits.push(SearchHit::empty_hint(&format!(
            "{overflow} more candidate symbols in {to_path} truncated; use `outline of {to_path}` for the full list"
        )));
    }

    hits
}

/// Upper bound on the candidate-endpoint list in `chain_to_file_path`
/// responses. Matches the per-query cap convention used elsewhere.
const CHAIN_FILE_CANDIDATE_CAP: usize = 10;

/// Heuristic: does `s` look like a file path rather than a symbol name?
/// True when `s` contains `/` or `\`, or when its trailing segment ends in
/// a known source extension. Deliberately conservative so bare identifiers
/// and qualified names like `module::fn` never trip this.
fn is_path_like(s: &str) -> bool {
    const EXTS: &[&str] = &[".rs", ".ts", ".tsx", ".js", ".jsx", ".py"];
    if s.contains('/') || s.contains('\\') {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    EXTS.iter().any(|ext| lower.ends_with(ext))
}

/// Canonical `SymbolId` for the synthetic "import source" module anchor the
/// parsers emit from. Every `emit_use` / `emit_import` across rust/ts/js/py
/// uses `{ file, name = file_stem, kind = Module }` as the `from` of an
/// Imports edge, so this gives us an O(1) key into `forward_edges`.
fn module_source_id(file: &std::path::Path) -> SymbolId {
    let name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_owned();
    SymbolId {
        file: file.to_path_buf(),
        name,
        kind: crate::graph::types::SymbolKind::Module,
    }
}

/// Canonical `SymbolId` for the synthetic "import target" module anchor. After
/// `resolve_imports` rewrites the raw spec to the resolved file path, every
/// Imports edge's `to` is `{ file: resolved, name = "", kind = Module }`.
fn module_target_id(file: &std::path::Path) -> SymbolId {
    SymbolId {
        file: file.to_path_buf(),
        name: String::new(),
        kind: crate::graph::types::SymbolKind::Module,
    }
}

/// `imports of FILE` — files that `file` imports (forward Imports edges).
///
/// O(1) hashmap lookup via the canonical module-source id. Was `O(total_edges)`
/// before the rewrite — a full-graph scan that dominated on large monorepos.
///
/// `project_root` is used to render each hit's target path relative.
#[must_use]
pub fn imports_of(
    graph: &CodeGraph,
    file: &std::path::Path,
    project_root: &std::path::Path,
) -> Vec<SearchHit> {
    let source = module_source_id(file);
    let mut hits = Vec::new();
    if let Some(edges) = graph.forward_edges.get(&source) {
        for e in edges {
            // Certain-only: unresolved imports point at raw spec text
            // (`crate::missing`, `super::*`) — filtering them here stops
            // parser internals leaking into the response.
            if e.kind == EdgeKind::Imports && e.confidence == Confidence::Certain {
                let rel = e.to.file.strip_prefix(project_root).unwrap_or(&e.to.file);
                hits.push(SearchHit {
                    file: e.to.file.clone(),
                    line: e.line,
                    signature: Some(format!("imports {}", rel.display())),
                    snippet: None,
                    context: None,
                });
            }
        }
    }
    if hits.is_empty() {
        return vec![SearchHit::empty_hint(
            "no internal imports in Phase 1 graph; use grep for \"use X::\"",
        )];
    }
    hits
}

/// `importers of FILE` — files that import `file` (reverse Imports edges).
///
/// O(1) hashmap lookup via the canonical module-target id. Was `O(total_edges)`
/// before the rewrite. Called inside `callers_of` and `tests_for` so the
/// speedup compounds on every agent query.
///
/// `project_root` is used to render the target path relative in each hit's
/// signature.
#[must_use]
pub fn importers_of(
    graph: &CodeGraph,
    file: &std::path::Path,
    project_root: &std::path::Path,
) -> Vec<SearchHit> {
    let target = module_target_id(file);
    let mut hits = Vec::new();
    let Some(edges) = graph.reverse_edges.get(&target) else {
        return hits;
    };
    for e in edges {
        if e.kind == EdgeKind::Imports && e.confidence == Confidence::Certain {
            let rel = e.to.file.strip_prefix(project_root).unwrap_or(&e.to.file);
            hits.push(SearchHit {
                file: e.from.file.clone(),
                line: e.line,
                signature: Some(format!("imports {}", rel.display())),
                snippet: None,
                context: None,
            });
        }
    }
    hits
}

/// `libraries` — external imports grouped by library name with use counts.
/// Returns results sorted alphabetically by library name (`BTreeMap` iteration).
///
/// Filters out names that correspond to project-internal packages:
/// - top-level subdirectories under `project_root` (e.g. `bench/` in a
///   mixed Rust+Python repo would otherwise show up as "bench library")
/// - the crate's own name from `Cargo.toml` (when present) — integration
///   tests doing `use blastguard::*` would otherwise self-report as an
///   external dependency
///
/// Both filters are silent — internal imports are not misclassified, they
/// just don't appear in the `libraries` output.
#[must_use]
pub fn libraries(graph: &CodeGraph, project_root: &std::path::Path) -> Vec<SearchHit> {
    use std::collections::BTreeMap;
    let own_crate_name = read_cargo_package_name(project_root);
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for li in &graph.library_imports {
        if is_internal_package(&li.library, project_root, own_crate_name.as_deref()) {
            continue;
        }
        *counts.entry(li.library.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(lib, count)| SearchHit {
            file: std::path::PathBuf::new(),
            line: 0,
            signature: Some(format!("{lib} ({count} uses)")),
            snippet: None,
            context: None,
        })
        .collect()
}

fn is_internal_package(
    library: &str,
    project_root: &std::path::Path,
    own_crate_name: Option<&str>,
) -> bool {
    if own_crate_name == Some(library) {
        return true;
    }
    project_root.join(library).is_dir()
}

/// Best-effort read of `[package].name` from `Cargo.toml` at `project_root`.
/// Returns `None` when the file is missing, unreadable, or malformed —
/// callers treat the absence as "no crate-name filter".
fn read_cargo_package_name(project_root: &std::path::Path) -> Option<String> {
    let manifest = project_root.join("Cargo.toml");
    let body = std::fs::read_to_string(&manifest).ok()?;
    let parsed: toml::Value = toml::from_str(&body).ok()?;
    parsed
        .get("package")?
        .get("name")?
        .as_str()
        .map(str::to_owned)
}

/// Heuristic: a path is a "test path" if any component contains `.test.`,
/// `.spec.`, `_test`, `test_`, or equals `tests` / `__tests__`.
fn is_test_path(path: &std::path::Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "tests"
            || s == "__tests__"
            || s.contains(".test.")
            || s.contains(".spec.")
            || s.contains("_test")
            || s.starts_with("test_")
    })
}

/// `tests for X` — if X contains a path separator treat it as a file, else
/// resolve X as a symbol name to its declaring file. Returns importers of
/// that file whose path is a test path.
#[must_use]
pub fn tests_for(
    graph: &CodeGraph,
    target: &str,
    project_root: &std::path::Path,
) -> Vec<SearchHit> {
    let target_file = if target.contains('/') || target.contains('\\') {
        // Use as-is — callers passing a relative query path (e.g.
        // "src/handler.ts") expect it to match the graph's storage
        // convention exactly. The dispatcher normalises absolute/relative
        // query paths before getting here.
        std::path::PathBuf::from(target)
    } else {
        let ids = find_by_name(graph, target);
        let Some(&id) = ids.first() else {
            return vec![SearchHit::empty_hint(&format!(
                "no same-file tests found; use grep for 'test_{target}'"
            ))];
        };
        id.file.clone()
    };

    let hits: Vec<SearchHit> = importers_of(graph, &target_file, project_root)
        .into_iter()
        .filter(|hit| is_test_path(&hit.file))
        .collect();
    if hits.is_empty() {
        return vec![SearchHit::empty_hint(&format!(
            "no same-file tests found; use grep for 'test_{target}'"
        ))];
    }
    hits
}

/// `exports of FILE` — visibility-filtered symbols declared in `file`.
#[must_use]
pub fn exports_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    use crate::graph::types::Visibility;
    let Some(sym_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    sym_ids
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .filter(|s| matches!(s.visibility, Visibility::Export))
        .map(SearchHit::structural)
        .collect()
}

/// `outline of FILE` — all symbols declared in `file`, sorted by `line_start`.
///
/// Duplicate-name entries after the first occurrence are prefixed with
/// `[test]` — files with both a production `fn foo` and a
/// `#[cfg(test)] fn foo` emit both; the later one is almost always the
/// test copy, so we tag it so agents can filter at a glance.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn outline_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let Some(symbol_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    let symbols: Vec<&Symbol> = symbol_ids
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .filter(|s| {
            matches!(
                s.id.kind,
                crate::graph::types::SymbolKind::Function
                    | crate::graph::types::SymbolKind::AsyncFunction
                    | crate::graph::types::SymbolKind::Method
            )
        })
        .collect();

    // Build hits and track which are methods and which are private helpers
    // for the collapse passes below.
    let mut hits: Vec<SearchHit> = symbols
        .iter()
        .map(|s| SearchHit::structural(s).without_return_type())
        .collect();
    let mut is_method: Vec<bool> = symbols
        .iter()
        .map(|s| s.id.kind == crate::graph::types::SymbolKind::Method)
        .collect();
    let mut is_private_fn: Vec<bool> = symbols
        .iter()
        .map(|s| {
            s.id.kind != crate::graph::types::SymbolKind::Method
                && s.visibility == crate::graph::types::Visibility::Private
        })
        .collect();
    // Sort all three in parallel by line.
    {
        let mut indices: Vec<usize> = (0..hits.len()).collect();
        indices.sort_by_key(|&i| hits[i].line);
        hits = indices.iter().map(|&i| hits[i].clone()).collect();
        is_method = indices.iter().map(|&i| is_method[i]).collect();
        is_private_fn = indices.iter().map(|&i| is_private_fn[i]).collect();
    }

    // Tag duplicate-name entries after the first occurrence as `[test]`.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for hit in &mut hits {
        let Some(sig) = &hit.signature else {
            continue;
        };
        let name = extract_fn_name(sig);
        if !seen.insert(name) {
            let tagged = format!("[test] {sig}");
            hit.signature = Some(tagged);
        }
    }

    // Collapse consecutive method entries (SymbolKind::Method) into a single
    // `impl { (N methods) }` summary entry. Methods make up 30-50% of public
    // symbols in Rust files but carry minimal orientation value — agents outline
    // to find types, then drill into specific methods via `find` or `callers of`.
    // This pass runs BEFORE test-function collapse so the method-kind tracking
    // stays parallel to the pre-collapse hits vec.
    let mut collapsed: Vec<SearchHit> = Vec::with_capacity(hits.len());
    let mut i = 0;
    while i < hits.len() {
        if is_method[i] {
            let group_start = i;
            let mut count = 1;
            i += 1;
            while i < hits.len() && is_method[i] {
                count += 1;
                i += 1;
            }
            if count >= 2 {
                let first = &hits[group_start];
                collapsed.push(SearchHit {
                    signature: Some(format!("impl {{ ({count} methods) }}")),
                    ..first.clone()
                });
            } else {
                collapsed.push(hits[group_start].clone());
            }
        } else {
            collapsed.push(hits[i].clone());
            i += 1;
        }
    }
    let hits = collapsed;

    // Collapse consecutive private-helper entries (non-method functions with
    // Visibility::Private) into a single `[helpers] name1, name2, ...` summary
    // entry. Unlike method/test collapse, private helpers are often what the
    // agent needs to enumerate by category — hiding them behind a bare count
    // causes the agent to fall back to expensive bash+grep calls to discover
    // the names. Showing names directly (~150 chars for 28 helpers) eliminates
    // that distrust-driven fallback.
    let mut collapsed: Vec<SearchHit> = Vec::with_capacity(hits.len());
    let mut i = 0;
    while i < hits.len() {
        if is_private_fn[i] {
            let group_start = i;
            let mut count = 1;
            i += 1;
            while i < hits.len() && is_private_fn[i] {
                count += 1;
                i += 1;
            }
            if count >= 2 {
                let first = &hits[group_start];
                let names: Vec<String> = hits[group_start..group_start + count]
                    .iter()
                    .filter_map(|h| h.signature.as_deref().map(extract_fn_name))
                    .collect();
                collapsed.push(SearchHit {
                    signature: Some(format!("[helpers] {}", names.join(", "))),
                    ..first.clone()
                });
            } else {
                collapsed.push(hits[group_start].clone());
            }
        } else {
            collapsed.push(hits[i].clone());
            i += 1;
        }
    }
    let hits = collapsed;

    // Collapse consecutive test-function entries (names starting with `test_`)
    // into a single `[test] (N functions)` summary entry. Test functions make
    // up 30-60% of public symbols in Rust test-heavy files but carry minimal
    // orientation value — agents outline to understand the API surface, then
    // call specific tests by name via `find`.
    let mut collapsed: Vec<SearchHit> = Vec::with_capacity(hits.len());
    let mut i = 0;
    while i < hits.len() {
        let Some(sig) = hits[i].signature.as_deref() else {
            collapsed.push(hits[i].clone());
            i += 1;
            continue;
        };
        let name = extract_fn_name(sig);
        if name.starts_with("test_") {
            let group_start = i;
            let mut count = 1;
            i += 1;
            while i < hits.len() {
                let Some(next_sig) = hits[i].signature.as_deref() else {
                    break;
                };
                let next_name = extract_fn_name(next_sig);
                if !next_name.starts_with("test_") {
                    break;
                }
                count += 1;
                i += 1;
            }
            if count >= 2 {
                let first = &hits[group_start];
                collapsed.push(SearchHit {
                    signature: Some(format!("[test] ({count} functions)")),
                    ..first.clone()
                });
            } else {
                collapsed.push(hits[group_start].clone());
            }
        } else {
            collapsed.push(hits[i].clone());
            i += 1;
        }
    }

    // Build a summary header showing total function count and category breakdown.
    // Gives the agent immediate confidence the outline is complete, reducing
    // distrust-driven read_file fallback (saves ~2k tokens on high-variance seeds).
    let mut regular = 0u32;
    let mut methods = 0u32;
    let mut tests = 0u32;
    let mut helpers = 0u32;
    for h in &collapsed {
        let Some(sig) = h.signature.as_deref() else {
            regular += 1;
            continue;
        };
        if sig.starts_with("impl { (") {
            methods += 1;
        } else if sig.starts_with("[test]") {
            tests += 1;
        } else if sig.starts_with("[helpers]") {
            helpers += 1;
        } else {
            regular += 1;
        }
    }
    let total = regular + methods + tests + helpers;
    let mut parts: Vec<String> = Vec::new();
    if regular > 0 {
        parts.push(format!("{regular} functions"));
    }
    if methods > 0 {
        parts.push(format!("{methods} method groups"));
    }
    if tests > 0 {
        parts.push(format!("{tests} test groups"));
    }
    if helpers > 0 {
        parts.push(format!("{helpers} helper groups"));
    }
    let summary = format!("{total} entries ({})", parts.join(", "));
    let mut result = Vec::with_capacity(collapsed.len() + 1);
    result.push(SearchHit::empty_hint(&summary));
    result.extend(collapsed);
    result
}

/// Extract the bare function name from a signature string for dedup purposes.
/// Takes the token before the first `(`, strips a leading `fn ` keyword.
fn extract_fn_name(sig: &str) -> String {
    let head = sig.split('(').next().unwrap_or(sig);
    let trimmed = head.strip_prefix("fn ").unwrap_or(head);
    trimmed.split_whitespace().last().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Symbol, SymbolId, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 10,
            line_end: 20,
            signature: format!("fn {name}(x: i32)"),
            params: vec!["x: i32".to_string()],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    fn sym_at(name: &str, file: &str, line: u32) -> Symbol {
        let mut s = sym(name, file);
        s.line_start = line;
        s
    }

    fn insert_with_centrality(graph: &mut CodeGraph, s: Symbol, centrality: u32) {
        let id = s.id.clone();
        graph.insert_symbol(s);
        graph.centrality.insert(id, centrality);
    }

    #[test]
    fn find_returns_exact_match_with_inline_signature() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("process", "a.ts"), 5);

        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 2, "count header + 1 match");
        assert!(hits[0].is_hint());
        assert!(hits[0].signature.as_deref().unwrap().contains("1 symbol"));
        assert_eq!(hits[1].file, PathBuf::from("a.ts"));
        assert_eq!(hits[1].line, 10);
        assert_eq!(hits[1].signature.as_deref(), Some("fn process(x: i32)"));
        assert!(hits[1].snippet.is_none());
    }

    #[test]
    fn find_fuzzy_match_when_no_exact() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("procss", "b.ts"), 1);

        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 2, "count header + 1 match");
        assert!(hits[0].is_hint());
        assert!(hits[0].signature.as_deref().unwrap().contains("1 symbol"));
        assert_eq!(hits[1].signature.as_deref(), Some("fn procss(x: i32)"));
    }

    #[test]
    fn find_sorts_by_centrality_descending() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("process", "low.ts"), 1);
        insert_with_centrality(&mut g, sym("process", "high.ts"), 100);

        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 3, "count header + 2 matches");
        assert!(hits[0].is_hint());
        assert!(hits[0].signature.as_deref().unwrap().contains("2 symbols"));
        assert_eq!(hits[1].file, PathBuf::from("high.ts"));
        assert_eq!(hits[2].file, PathBuf::from("low.ts"));
    }

    #[test]
    fn find_caps_at_max_hits() {
        let mut g = CodeGraph::new();
        for i in 0..20 {
            insert_with_centrality(&mut g, sym("dup", &format!("f{i}.ts")), i);
        }
        let hits = find(&g, "dup", 5);
        assert_eq!(hits.len(), 6, "count header + 5 hits");
    }

    #[test]
    fn find_empty_when_no_match_at_all() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("process", "a.ts"), 0);
        let hits = find(&g, "xyz_no_match_anywhere", 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].is_hint());
        assert!(hits[0].signature.as_deref().unwrap().contains("grep"));
    }

    #[test]
    fn callers_of_returns_callers_with_signatures() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};

        let mut g = CodeGraph::new();
        let target = sym("target", "t.ts");
        let caller_a = sym("caller_a", "a.ts");
        let caller_b = sym("caller_b", "b.ts");
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller_a.clone(), 0);
        insert_with_centrality(&mut g, caller_b.clone(), 0);

        for caller in [&caller_a, &caller_b] {
            g.insert_edge(Edge {
                from: caller.id.clone(),
                to: target.id.clone(),
                kind: EdgeKind::Calls,
                line: 5,
                confidence: Confidence::Certain,
            });
        }

        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), false);
        let files: Vec<_> = hits.iter().map(|h| h.file.clone()).collect();
        assert!(files.contains(&PathBuf::from("a.ts")));
        assert!(files.contains(&PathBuf::from("b.ts")));
        assert!(hits.iter().all(|h| h.signature.is_some()));
    }

    #[test]
    fn callers_of_skips_importer_hint_when_function_caller_from_same_file() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};

        let mut g = CodeGraph::new();
        // login() lives in auth.ts.
        let target = sym("login", "auth.ts");
        // handle() in handler.ts calls login() — first-class Calls edge.
        let caller = sym("handle", "handler.ts");
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Inferred,
        });
        // AND an Imports edge from handler.ts → auth.ts (would otherwise
        // produce an importer hint pointing at the same file).
        g.insert_edge(Edge {
            from: SymbolId {
                file: PathBuf::from("handler.ts"),
                name: "handler".to_owned(),
                kind: SymbolKind::Module,
            },
            to: SymbolId {
                file: PathBuf::from("auth.ts"),
                name: String::new(),
                kind: SymbolKind::Module,
            },
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Certain,
        });

        let hits = callers_of(&g, "login", 10, std::path::Path::new("."), false);
        // Hit 0 is the count header, hit 1 is the function-level caller.
        assert_eq!(
            hits.len(),
            2,
            "expected 2 hits (header + function caller, no duplicate importer hint); got {hits:?}"
        );
        assert_eq!(hits[1].file, PathBuf::from("handler.ts"));
        assert!(
            !hits[1]
                .signature
                .as_deref()
                .is_some_and(|s| s.contains("cross-file importer")),
            "hit 1 must be the function caller, not the importer hint: {:?}",
            hits[1].signature
        );
    }

    #[test]
    fn callers_of_empty_when_target_missing() {
        let g = CodeGraph::new();
        let hits = callers_of(&g, "nonexistent", 10, std::path::Path::new("."), false);
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("nonexistent")));
    }

    #[test]
    fn callers_of_caps_at_max_hits() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};

        let mut g = CodeGraph::new();
        let target = sym("target", "t.ts");
        insert_with_centrality(&mut g, target.clone(), 0);

        for i in 0..15 {
            let caller = sym(&format!("c{i}"), &format!("c{i}.ts"));
            insert_with_centrality(&mut g, caller.clone(), 0);
            g.insert_edge(Edge {
                from: caller.id.clone(),
                to: target.id.clone(),
                kind: EdgeKind::Calls,
                line: 1,
                confidence: Confidence::Certain,
            });
        }

        let hits = callers_of(&g, "target", 5, std::path::Path::new("."), false);
        // 1 count header + 5 caller hits = 6
        assert_eq!(hits.len(), 6);
    }

    #[test]
    fn callees_of_returns_forward_edges() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let caller = sym("caller", "a.ts");
        let callee_a = sym("helper_a", "b.ts");
        let callee_b = sym("helper_b", "c.ts");
        insert_with_centrality(&mut g, caller.clone(), 0);
        insert_with_centrality(&mut g, callee_a.clone(), 0);
        insert_with_centrality(&mut g, callee_b.clone(), 0);
        for callee in [&callee_a, &callee_b] {
            g.insert_edge(Edge {
                from: caller.id.clone(),
                to: callee.id.clone(),
                kind: EdgeKind::Calls,
                line: 5,
                confidence: Confidence::Certain,
            });
        }
        let hits = callees_of(&g, "caller", 10);
        let files: Vec<_> = hits.iter().map(|h| h.file.clone()).collect();
        assert!(files.contains(&PathBuf::from("b.ts")));
        assert!(files.contains(&PathBuf::from("c.ts")));
    }

    #[test]
    fn chain_from_to_returns_shortest_path() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let a = sym("a", "a.ts");
        let b = sym("b", "b.ts");
        let c = sym("c", "c.ts");
        insert_with_centrality(&mut g, a.clone(), 0);
        insert_with_centrality(&mut g, b.clone(), 0);
        insert_with_centrality(&mut g, c.clone(), 0);
        for (from, to) in [(&a, &b), (&b, &c)] {
            g.insert_edge(Edge {
                from: from.id.clone(),
                to: to.id.clone(),
                kind: EdgeKind::Calls,
                line: 1,
                confidence: Confidence::Certain,
            });
        }
        let hits = chain_from_to(&g, "a", "c");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].file, PathBuf::from("a.ts"));
        assert_eq!(hits[2].file, PathBuf::from("c.ts"));
    }

    #[test]
    fn chain_from_to_falls_back_to_endpoints_plus_hint_when_unreachable() {
        let mut g = CodeGraph::new();
        let a = sym("a", "a.ts");
        let b = sym("b", "b.ts");
        insert_with_centrality(&mut g, a, 0);
        insert_with_centrality(&mut g, b, 0);
        let hits = chain_from_to(&g, "a", "b");
        // Two endpoint hits + one hint — never empty when both symbols exist.
        assert_eq!(hits.len(), 3, "got: {hits:?}");
        assert!(hits.iter().any(|h| h.file == std::path::Path::new("a.ts")));
        assert!(hits.iter().any(|h| h.file == std::path::Path::new("b.ts")));
        assert!(hits.iter().any(|h| h
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("no call-graph path"))));
    }

    #[test]
    fn chain_from_to_empty_hint_when_from_symbol_missing() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("b", "b.ts"), 0);
        let hits = chain_from_to(&g, "a_nonexistent", "b");
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("a_nonexistent")));
    }

    #[test]
    fn outline_of_returns_all_symbols_in_file_sorted_by_line() {
        let mut g = CodeGraph::new();
        let a = sym_at("a", "x.ts", 10);
        let b = sym_at("b", "x.ts", 5);
        let c = sym_at("c", "y.ts", 1);
        for s in [a, b, c] {
            g.insert_symbol(s);
        }
        let hits = outline_of(&g, std::path::Path::new("x.ts"));
        assert_eq!(hits.len(), 3, "summary header + 2 symbols");
        assert!(hits[0].is_hint(), "first hit is the summary header");
        assert_eq!(hits[1].line, 5);
        assert_eq!(hits[2].line, 10);
        assert!(hits[1..]
            .iter()
            .all(|h| h.file == std::path::Path::new("x.ts")));
    }

    #[test]
    fn outline_of_empty_when_no_file_symbols() {
        let g = CodeGraph::new();
        let hits = outline_of(&g, std::path::Path::new("nope.ts"));
        assert!(hits.is_empty());
    }

    #[test]
    fn outline_of_includes_all_visibility_symbols() {
        let mut g = CodeGraph::new();
        let mut pub_sym = sym("public_fn", "x.rs");
        pub_sym.visibility = Visibility::Export;
        let mut priv_sym = sym_at("private_fn", "x.rs", 20);
        priv_sym.visibility = Visibility::Private;
        g.insert_symbol(pub_sym);
        g.insert_symbol(priv_sym);
        let hits = outline_of(&g, std::path::Path::new("x.rs"));
        assert_eq!(
            hits.len(),
            3,
            "summary header + all symbols regardless of visibility should be included"
        );
        assert!(hits[0].is_hint(), "first hit is the summary header");
        assert!(hits[1]
            .signature
            .as_deref()
            .unwrap_or("")
            .contains("public_fn"));
        assert!(hits[2]
            .signature
            .as_deref()
            .unwrap_or("")
            .contains("private_fn"));
    }

    #[test]
    fn imports_of_and_importers_of_traverse_imports_edges() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        let a_id = SymbolId {
            file: PathBuf::from("a.ts"),
            name: "a".to_string(),
            kind: SymbolKind::Module,
        };
        // Empty name matches the parser's canonical import-target shape
        // (emit_import sets `to.name = String::new()`). The O(1) lookup in
        // `importers_of` keys on this convention.
        let b_id = SymbolId {
            file: PathBuf::from("b.ts"),
            name: String::new(),
            kind: SymbolKind::Module,
        };
        // Insert stub modules so file_symbols is populated.
        let mut mod_a = sym("a", "a.ts");
        mod_a.id = a_id.clone();
        let mut mod_b = sym("b", "b.ts");
        mod_b.id = b_id.clone();
        g.insert_symbol(mod_a);
        g.insert_symbol(mod_b);
        g.insert_edge(Edge {
            from: a_id.clone(),
            to: b_id.clone(),
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Certain,
        });

        let imports = imports_of(&g, std::path::Path::new("a.ts"), std::path::Path::new("."));
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].file, PathBuf::from("b.ts"));

        let importers = importers_of(&g, std::path::Path::new("b.ts"), std::path::Path::new("."));
        assert_eq!(importers.len(), 1);
        assert_eq!(importers[0].file, PathBuf::from("a.ts"));
    }

    #[test]
    fn tests_for_file_filters_importers_to_test_paths_only() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        // src_id is the edge TARGET; parser convention is empty name.
        let src_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: String::new(),
            kind: SymbolKind::Module,
        };
        let test_id = SymbolId {
            file: PathBuf::from("tests/handler.test.ts"),
            name: "test_handler".to_string(),
            kind: SymbolKind::Module,
        };
        let other_id = SymbolId {
            file: PathBuf::from("src/other.ts"),
            name: "other".to_string(),
            kind: SymbolKind::Module,
        };
        for id in [&src_id, &test_id, &other_id] {
            let mut s = sym(&id.name, &id.file.to_string_lossy());
            s.id = id.clone();
            g.insert_symbol(s);
        }
        for from in [&test_id, &other_id] {
            g.insert_edge(Edge {
                from: from.clone(),
                to: src_id.clone(),
                kind: EdgeKind::Imports,
                line: 1,
                confidence: Confidence::Certain,
            });
        }
        let hits = tests_for(&g, "src/handler.ts", std::path::Path::new("."));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.to_string_lossy().contains(".test."));
    }

    #[test]
    fn tests_for_symbol_name_resolves_to_file_first() {
        let mut g = CodeGraph::new();
        g.insert_symbol(sym("processRequest", "src/handler.ts"));
        let hits = tests_for(&g, "processRequest", std::path::Path::new("."));
        // No importers at all → hint; just verifies it doesn't panic.
        assert_eq!(hits.len(), 1);
        assert!(hits[0]
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("grep")));
    }

    #[test]
    fn tests_for_recognises_double_underscore_tests_dir() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        let src_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: String::new(),
            kind: SymbolKind::Module,
        };
        let jest_test_id = SymbolId {
            file: PathBuf::from("src/__tests__/handler.ts"),
            name: "jest_handler".to_string(),
            kind: SymbolKind::Module,
        };
        for id in [&src_id, &jest_test_id] {
            let mut s = sym(&id.name, &id.file.to_string_lossy());
            s.id = id.clone();
            g.insert_symbol(s);
        }
        g.insert_edge(Edge {
            from: jest_test_id.clone(),
            to: src_id.clone(),
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = tests_for(&g, "src/handler.ts", std::path::Path::new("."));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.to_string_lossy().contains("__tests__"));
    }

    #[test]
    fn outline_of_tags_duplicate_test_functions() {
        let file = PathBuf::from("/p/a.rs");
        let mut g = CodeGraph::new();
        // Production function at line 10 (Function kind).
        g.insert_symbol(sym_at("foo", "/p/a.rs", 10));
        // Test function at line 100 — uses Method kind to give it a distinct
        // SymbolId so the HashMap doesn't overwrite the first entry. The
        // dedup logic keys on the extracted name from the signature string,
        // so different SymbolKinds with the same name still trigger tagging.
        let mut test_sym = sym_at("foo", "/p/a.rs", 100);
        test_sym.id.kind = SymbolKind::Method;
        g.insert_symbol(test_sym);
        let hits = outline_of(&g, &file);
        assert_eq!(hits.len(), 3, "summary header + 2 symbols");
        assert!(hits[0].is_hint(), "first hit is the summary header");
        // The later one should be tagged.
        let tagged = hits.iter().find(|h| h.line == 100).expect("hit at 100");
        assert!(
            tagged
                .signature
                .as_deref()
                .unwrap_or("")
                .starts_with("[test]"),
            "expected [test] tag, got: {:?}",
            tagged.signature
        );
        // The earlier one should NOT be tagged.
        let untagged = hits.iter().find(|h| h.line == 10).expect("hit at 10");
        assert!(!untagged
            .signature
            .as_deref()
            .unwrap_or("")
            .starts_with("[test]"));
    }

    #[test]
    fn tests_for_recognises_spec_suffix() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        let src_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: String::new(),
            kind: SymbolKind::Module,
        };
        let spec_id = SymbolId {
            file: PathBuf::from("src/handler.spec.ts"),
            name: "spec_handler".to_string(),
            kind: SymbolKind::Module,
        };
        for id in [&src_id, &spec_id] {
            let mut s = sym(&id.name, &id.file.to_string_lossy());
            s.id = id.clone();
            g.insert_symbol(s);
        }
        g.insert_edge(Edge {
            from: spec_id.clone(),
            to: src_id.clone(),
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = tests_for(&g, "src/handler.ts", std::path::Path::new("."));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.to_string_lossy().contains(".spec."));
    }

    #[test]
    fn callers_of_returns_hint_when_no_callers_found() {
        let mut g = CodeGraph::new();
        g.insert_symbol(sym("orphan", "a.rs"));
        let hits = callers_of(&g, "orphan", 10, std::path::Path::new("."), false);
        assert_eq!(hits.len(), 1);
        let hint = hits[0].signature.as_deref().expect("hint signature");
        assert!(hint.contains("cross-file"));
        assert!(hint.contains("grep"));
    }

    #[test]
    fn callers_of_appends_cross_file_importer_hints() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        // Target `foo` in target.rs. No intra-file callers. Two other files
        // import target.rs. callers_of should return hints for both
        // importers instead of a bare "no callers" message.
        let mut g = CodeGraph::new();
        let target_file = PathBuf::from("src/target.rs");
        let importer_a = PathBuf::from("src/a.rs");
        let importer_b = PathBuf::from("src/b.rs");

        g.insert_symbol(sym("foo", "src/target.rs"));
        // Module-symbol stubs so importers_of can attach edges to them.
        for file in [&target_file, &importer_a, &importer_b] {
            let id = SymbolId {
                file: file.clone(),
                name: file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("mod")
                    .to_string(),
                kind: SymbolKind::Module,
            };
            let mut s = sym(&id.name, &id.file.to_string_lossy());
            s.id = id;
            g.insert_symbol(s);
        }
        // a.rs and b.rs both import target.rs.
        for importer in [&importer_a, &importer_b] {
            g.insert_edge(Edge {
                from: SymbolId {
                    file: importer.clone(),
                    name: importer.file_stem().unwrap().to_string_lossy().into_owned(),
                    kind: SymbolKind::Module,
                },
                to: SymbolId {
                    file: target_file.clone(),
                    name: String::new(),
                    kind: SymbolKind::Module,
                },
                kind: EdgeKind::Imports,
                line: 1,
                confidence: Confidence::Certain,
            });
        }

        let hits = callers_of(&g, "foo", 10, std::path::Path::new("."), false);
        // Hit 0 is the count header, hits 1-2 are the importer hints.
        assert_eq!(
            hits.len(),
            3,
            "expected 3 hits (header + two cross-file importer hints), got {:?}",
            hits.iter().map(|h| &h.file).collect::<Vec<_>>()
        );
        // Skip the header (index 0), check the importer hints at 1-2.
        let importer_hits: Vec<&SearchHit> = hits.iter().skip(1).collect();
        let importer_files: std::collections::HashSet<_> =
            importer_hits.iter().map(|h| &h.file).collect();
        assert!(importer_files.contains(&importer_a));
        assert!(importer_files.contains(&importer_b));
        for hit in &importer_hits {
            let sig = hit.signature.as_deref().unwrap_or("");
            assert!(
                sig.contains("cross-file importer"),
                "importer hint signature wrong: {sig}"
            );
            assert!(sig.contains("foo"), "hint should name symbol `foo`: {sig}");
        }
    }

    #[test]
    fn find_and_callers_of_rank_by_centrality_consistently() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};

        let mut g = CodeGraph::new();
        let target = sym("target", "t.ts");
        let hi = sym("hi", "hi.ts");
        let lo = sym("lo", "lo.ts");

        // find() ranks name matches by the SYMBOL'S centrality.
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, hi.clone(), 100);
        insert_with_centrality(&mut g, lo.clone(), 1);

        // callers_of() ranks callers by the CALLER'S centrality.
        for caller in [&hi, &lo] {
            g.insert_edge(Edge {
                from: caller.id.clone(),
                to: target.id.clone(),
                kind: EdgeKind::Calls,
                line: 1,
                confidence: Confidence::Certain,
            });
        }

        // `find hi` returns the high-centrality hit first (header + 1 match).
        let find_hits = find(&g, "hi", 10);
        assert_eq!(find_hits.len(), 2, "count header + 1 match");
        assert!(find_hits[0].is_hint());
        assert_eq!(find_hits[1].file, PathBuf::from("hi.ts"));

        // `callers of target` orders hi before lo because hi has centrality 100.
        // Hit 0 is the count header, hits 1-2 are the callers.
        let caller_hits = callers_of(&g, "target", 10, std::path::Path::new("."), false);
        assert_eq!(caller_hits.len(), 3, "header + 2 callers");
        assert_eq!(caller_hits[1].file, PathBuf::from("hi.ts"));
        assert_eq!(caller_hits[2].file, PathBuf::from("lo.ts"));
    }

    #[test]
    fn exports_of_returns_only_exported_symbols() {
        use crate::graph::types::Visibility;
        let mut g = CodeGraph::new();
        let mut pub_sym = sym("api", "x.ts");
        pub_sym.visibility = Visibility::Export;
        let mut priv_sym = sym("internal", "x.ts");
        priv_sym.visibility = Visibility::Private;
        g.insert_symbol(pub_sym);
        g.insert_symbol(priv_sym);
        let hits = exports_of(&g, std::path::Path::new("x.ts"));
        assert_eq!(hits.len(), 1);
        assert!(hits[0].signature.as_deref().unwrap().contains("api"));
    }

    #[test]
    fn libraries_returns_unique_libraries_with_counts() {
        use crate::graph::types::LibraryImport;
        let mut g = CodeGraph::new();
        for (lib, file, line) in [
            ("lodash", "a.ts", 1),
            ("lodash", "b.ts", 1),
            ("@tanstack/react-query", "a.ts", 2),
            ("tokio", "lib.rs", 1),
        ] {
            g.library_imports.push(LibraryImport {
                library: lib.to_string(),
                symbol: String::new(),
                file: std::path::PathBuf::from(file),
                line,
            });
        }
        let hits = libraries(&g, std::path::Path::new("/nonexistent-for-filter"));
        assert_eq!(hits.len(), 3);
        let lodash_hit = hits
            .iter()
            .find(|h| h.signature.as_deref().is_some_and(|s| s.contains("lodash")))
            .expect("lodash missing");
        assert!(lodash_hit.signature.as_deref().unwrap().contains("2 uses"));
    }

    #[test]
    fn libraries_filters_project_internal_packages() {
        use crate::graph::types::LibraryImport;
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("bench")).expect("mkdir bench");
        // Write a Cargo.toml so the crate-name filter also kicks in.
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"myproj\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");

        let mut g = CodeGraph::new();
        for (lib, file) in [
            ("tokio", "src/a.rs"),
            ("bench", "bench/x.py"),  // internal subdir → filtered
            ("myproj", "tests/t.rs"), // own crate name → filtered
            ("anyhow", "src/b.rs"),
        ] {
            g.library_imports.push(LibraryImport {
                library: lib.to_string(),
                symbol: String::new(),
                file: std::path::PathBuf::from(file),
                line: 1,
            });
        }
        let hits = libraries(&g, tmp.path());
        let names: Vec<_> = hits
            .iter()
            .filter_map(|h| {
                h.signature
                    .as_deref()
                    .and_then(|s| s.split_whitespace().next())
            })
            .collect();
        assert_eq!(names, vec!["anyhow", "tokio"], "got: {names:?}");
    }

    #[test]
    fn chain_to_file_path_returns_chain_plus_file_candidates() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        // search_tool (mcp/server.rs) -> dispatch (search/dispatcher.rs)
        //   -> find (search/structural.rs)
        //   dispatch also calls callers_of in structural.rs directly.
        let search_tool = sym("search_tool", "src/mcp/server.rs");
        let dispatch = sym("dispatch", "src/search/dispatcher.rs");
        let find = sym("find", "src/search/structural.rs");
        let callers = sym("callers_of", "src/search/structural.rs");
        insert_with_centrality(&mut g, search_tool.clone(), 0);
        insert_with_centrality(&mut g, dispatch.clone(), 0);
        insert_with_centrality(&mut g, find.clone(), 5);
        insert_with_centrality(&mut g, callers.clone(), 2);
        for (from, to) in [
            (&search_tool, &dispatch),
            (&dispatch, &find),
            (&dispatch, &callers),
        ] {
            g.insert_edge(Edge {
                from: from.id.clone(),
                to: to.id.clone(),
                kind: EdgeKind::Calls,
                line: 1,
                confidence: Confidence::Certain,
            });
        }
        let hits = chain_from_to(&g, "search_tool", "src/search/structural.rs");
        // First three hits are the chain search_tool -> dispatch -> find.
        assert!(hits.len() >= 3, "expected chain + candidates, got {hits:?}");
        assert_eq!(hits[0].file, PathBuf::from("src/mcp/server.rs"));
        assert_eq!(hits[1].file, PathBuf::from("src/search/dispatcher.rs"));
        assert_eq!(hits[2].file, PathBuf::from("src/search/structural.rs"));
        // `callers_of` is a sibling candidate reached from a chain node.
        let candidate_names: Vec<&str> = hits
            .iter()
            .skip(3)
            .filter_map(|h| h.signature.as_deref())
            .collect();
        assert!(
            candidate_names.iter().any(|s| s.contains("callers_of")),
            "expected callers_of in candidates, got {candidate_names:?}"
        );
    }

    #[test]
    fn chain_to_file_path_backward_compat_with_symbol_name() {
        // A bare identifier must still route through the existing
        // name-to-name BFS — this mirrors chain_from_to_returns_shortest_path
        // and guards against the path-like heuristic overreaching.
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let a = sym("a", "a.ts");
        let b = sym("b", "b.ts");
        insert_with_centrality(&mut g, a.clone(), 0);
        insert_with_centrality(&mut g, b.clone(), 0);
        g.insert_edge(Edge {
            from: a.id.clone(),
            to: b.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = chain_from_to(&g, "a", "b");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file, PathBuf::from("a.ts"));
        assert_eq!(hits[1].file, PathBuf::from("b.ts"));
    }

    #[test]
    fn chain_to_file_path_unreachable_falls_back_with_hint() {
        let mut g = CodeGraph::new();
        // `from` exists, target file has an indexed symbol, but no call
        // edges connect them.
        insert_with_centrality(&mut g, sym("caller", "src/a.rs"), 0);
        insert_with_centrality(&mut g, sym("island", "src/unreachable.rs"), 0);
        let hits = chain_from_to(&g, "caller", "src/unreachable.rs");
        assert!(
            hits.iter()
                .any(|h| h.file == std::path::Path::new("src/a.rs")),
            "expected `from` hit, got {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h
                .signature
                .as_deref()
                .is_some_and(|s| s.contains("no call-graph path"))),
            "expected unreachable hint, got {hits:?}"
        );
    }

    #[test]
    fn chain_to_file_path_when_from_already_in_target_file() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let a = sym("a", "src/x.rs");
        let b = sym("b", "src/x.rs");
        insert_with_centrality(&mut g, a.clone(), 0);
        insert_with_centrality(&mut g, b.clone(), 0);
        g.insert_edge(Edge {
            from: a.id.clone(),
            to: b.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = chain_from_to(&g, "a", "src/x.rs");
        // Chain is just [a] (already in target), candidates include b.
        assert!(hits
            .iter()
            .any(|h| h.signature.as_deref() == Some("fn a(x: i32)")));
        assert!(hits
            .iter()
            .any(|h| h.signature.as_deref() == Some("fn b(x: i32)")));
    }

    #[test]
    fn chain_to_file_path_when_file_not_indexed() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("caller", "src/a.rs"), 0);
        let hits = chain_from_to(&g, "caller", "src/does_not_exist.rs");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].is_hint());
        assert!(hits[0]
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("no symbols indexed")));
    }

    #[test]
    fn callers_of_without_context_leaves_context_field_none() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let target = sym("target", "a.rs");
        let caller = sym("caller", "b.rs");
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });
        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), false);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert!(!real.is_empty());
        for h in real {
            assert!(
                h.context.is_none(),
                "expected no context, got: {:?}",
                h.context
            );
        }
    }

    #[test]
    fn callers_of_with_context_attaches_text() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        use std::io::Write;
        let tmpdir = tempfile::tempdir().expect("tmpdir");
        let caller_path = tmpdir.path().join("caller.rs");
        let caller_src = "fn caller() {\n    let _ = target(42, \"hello\");\n}\n";
        std::fs::File::create(&caller_path)
            .expect("create")
            .write_all(caller_src.as_bytes())
            .expect("write");

        let mut g = CodeGraph::new();
        let target = sym("target", tmpdir.path().join("a.rs").to_str().unwrap());
        let mut caller = sym("caller", caller_path.to_str().unwrap());
        caller.line_start = 1;
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 2,
            confidence: Confidence::Certain,
        });
        let hits = callers_of(&g, "target", 10, tmpdir.path(), true);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert_eq!(real.len(), 1);
        let ctx = real[0].context.as_deref().expect("context attached");
        assert!(
            ctx.contains("target(42, \"hello\")"),
            "expected call literal in context, got: {ctx}"
        );
    }

    #[test]
    fn callers_of_respects_limit_with_context() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let target = sym("target", "a.rs");
        insert_with_centrality(&mut g, target.clone(), 0);
        for i in 0..15u32 {
            let caller = sym(&format!("caller{i}"), "b.rs");
            insert_with_centrality(&mut g, caller.clone(), 0);
            g.insert_edge(Edge {
                from: caller.id.clone(),
                to: target.id.clone(),
                kind: EdgeKind::Calls,
                line: i + 1,
                confidence: Confidence::Certain,
            });
        }
        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), true);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert_eq!(
            real.len(),
            10,
            "limit=10 must be respected even with context"
        );
    }

    #[test]
    fn callers_of_with_context_degrades_gracefully_on_missing_file() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let target = sym("target", "does_not_exist_a.rs");
        let caller = sym("caller", "does_not_exist_b.rs");
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });
        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), true);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert!(!real.is_empty());
        // No panic. Context may be None because file can't be read.
        let _ = real[0].context.clone();
    }
}
