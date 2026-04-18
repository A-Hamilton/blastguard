//! Graph-backed search backends — SPEC §3.1.
//!
//! Each public function resolves a [`super::query::QueryKind`] arm against the
//! [`CodeGraph`] and renders hits via [`super::hit::SearchHit::structural`].

use crate::graph::ops::{callees, callers, find_by_name, shortest_path};
use crate::graph::types::{CodeGraph, EdgeKind, SymbolId};
use crate::search::hit::{sort_by_centrality, SearchHit};

/// `find X` / `where is X` — centrality-ranked name lookup with fuzzy fallback.
/// Returns at most `max_hits` results.
#[must_use]
pub fn find(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut ids: Vec<&SymbolId> = find_by_name(graph, name);
    sort_by_centrality(graph, &mut ids);
    ids.into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}

// TODO(plan-3): introduce callers_of_id / tests_for_id / importers_of_id
// helpers that take a pre-resolved &SymbolId. Plan 3's apply_change will
// already hold the exact id of the symbol it just wrote; re-resolving by
// name risks false positives on name collisions.

/// `callers of X` / `what calls X` — reverse BFS (1 hop) with inline signatures.
///
/// Resolves `name` to the most-central exact-match symbol, then returns its
/// direct callers sorted by their own centrality descending, capped at
/// `max_hits`.
#[must_use]
pub fn callers_of(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut targets = find_by_name(graph, name);
    if targets.is_empty() {
        return Vec::new();
    }
    sort_by_centrality(graph, &mut targets);
    let Some(&target_id) = targets.first() else {
        return Vec::new();
    };
    let mut caller_ids: Vec<&SymbolId> = callers(graph, target_id);
    sort_by_centrality(graph, &mut caller_ids);
    caller_ids
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
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

/// `chain from X to Y` — BFS shortest path, rendered as a sequence of hits.
#[must_use]
pub fn chain_from_to(graph: &CodeGraph, from_name: &str, to_name: &str) -> Vec<SearchHit> {
    let from_ids = find_by_name(graph, from_name);
    let to_ids = find_by_name(graph, to_name);
    let (Some(&from_id), Some(&to_id)) = (from_ids.first(), to_ids.first()) else {
        return Vec::new();
    };
    let Some(path) = shortest_path(graph, from_id, to_id) else {
        return Vec::new();
    };
    path.iter()
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}

/// `imports of FILE` — files that `file` imports (forward Imports edges).
#[must_use]
pub fn imports_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let Some(sym_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for sid in sym_ids {
        let Some(edges) = graph.forward_edges.get(sid) else {
            continue;
        };
        for e in edges {
            if e.kind == EdgeKind::Imports {
                hits.push(SearchHit {
                    file: e.to.file.clone(),
                    line: e.line,
                    signature: Some(format!("imports {}", e.to.file.display())),
                    snippet: None,
                });
            }
        }
    }
    hits
}

/// `importers of FILE` — files that import `file` (reverse Imports edges).
#[must_use]
pub fn importers_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for rev_edges in graph.reverse_edges.values() {
        for e in rev_edges {
            if e.kind == EdgeKind::Imports && e.to.file == file {
                hits.push(SearchHit {
                    file: e.from.file.clone(),
                    line: e.line,
                    signature: Some(format!("imports {}", e.to.file.display())),
                    snippet: None,
                });
            }
        }
    }
    hits
}

/// `libraries` — external imports grouped by library name with use counts.
/// Returns results sorted alphabetically by library name (`BTreeMap` iteration).
#[must_use]
pub fn libraries(graph: &CodeGraph) -> Vec<SearchHit> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for li in &graph.library_imports {
        *counts.entry(li.library.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(lib, count)| SearchHit {
            file: std::path::PathBuf::new(),
            line: 0,
            signature: Some(format!("{lib} ({count} uses)")),
            snippet: None,
        })
        .collect()
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
pub fn tests_for(graph: &CodeGraph, target: &str) -> Vec<SearchHit> {
    let target_file = if target.contains('/') || target.contains('\\') {
        std::path::PathBuf::from(target)
    } else {
        let ids = find_by_name(graph, target);
        let Some(&id) = ids.first() else {
            return Vec::new();
        };
        id.file.clone()
    };

    importers_of(graph, &target_file)
        .into_iter()
        .filter(|hit| is_test_path(&hit.file))
        .collect()
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
#[must_use]
pub fn outline_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let Some(symbol_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    let mut hits: Vec<SearchHit> = symbol_ids
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect();
    hits.sort_by_key(|h| h.line);
    hits
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
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, PathBuf::from("a.ts"));
        assert_eq!(hits[0].line, 10);
        assert_eq!(hits[0].signature.as_deref(), Some("fn process(x: i32)"));
        assert!(hits[0].snippet.is_none());
    }

    #[test]
    fn find_fuzzy_match_when_no_exact() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("procss", "b.ts"), 1);

        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].signature.as_deref(), Some("fn procss(x: i32)"));
    }

    #[test]
    fn find_sorts_by_centrality_descending() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("process", "low.ts"), 1);
        insert_with_centrality(&mut g, sym("process", "high.ts"), 100);

        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file, PathBuf::from("high.ts"));
        assert_eq!(hits[1].file, PathBuf::from("low.ts"));
    }

    #[test]
    fn find_caps_at_max_hits() {
        let mut g = CodeGraph::new();
        for i in 0..20 {
            insert_with_centrality(&mut g, sym("dup", &format!("f{i}.ts")), i);
        }
        let hits = find(&g, "dup", 5);
        assert_eq!(hits.len(), 5);
    }

    #[test]
    fn find_empty_when_no_match_at_all() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("process", "a.ts"), 0);
        let hits = find(&g, "xyz_no_match_anywhere", 10);
        assert!(hits.is_empty());
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

        let hits = callers_of(&g, "target", 10);
        let files: Vec<_> = hits.iter().map(|h| h.file.clone()).collect();
        assert!(files.contains(&PathBuf::from("a.ts")));
        assert!(files.contains(&PathBuf::from("b.ts")));
        assert!(hits.iter().all(|h| h.signature.is_some()));
    }

    #[test]
    fn callers_of_empty_when_target_missing() {
        let g = CodeGraph::new();
        let hits = callers_of(&g, "nonexistent", 10);
        assert!(hits.is_empty());
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

        let hits = callers_of(&g, "target", 5);
        assert_eq!(hits.len(), 5);
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
    fn chain_from_to_empty_when_unreachable() {
        let mut g = CodeGraph::new();
        let a = sym("a", "a.ts");
        let b = sym("b", "b.ts");
        insert_with_centrality(&mut g, a, 0);
        insert_with_centrality(&mut g, b, 0);
        let hits = chain_from_to(&g, "a", "b");
        assert!(hits.is_empty());
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
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 5);
        assert_eq!(hits[1].line, 10);
        assert!(hits.iter().all(|h| h.file == std::path::Path::new("x.ts")));
    }

    #[test]
    fn outline_of_empty_when_no_file_symbols() {
        let g = CodeGraph::new();
        let hits = outline_of(&g, std::path::Path::new("nope.ts"));
        assert!(hits.is_empty());
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
        let b_id = SymbolId {
            file: PathBuf::from("b.ts"),
            name: "b".to_string(),
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
            confidence: Confidence::Unresolved,
        });

        let imports = imports_of(&g, std::path::Path::new("a.ts"));
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].file, PathBuf::from("b.ts"));

        let importers = importers_of(&g, std::path::Path::new("b.ts"));
        assert_eq!(importers.len(), 1);
        assert_eq!(importers[0].file, PathBuf::from("a.ts"));
    }

    #[test]
    fn tests_for_file_filters_importers_to_test_paths_only() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        let src_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: "handler".to_string(),
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
        let hits = tests_for(&g, "src/handler.ts");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.to_string_lossy().contains(".test."));
    }

    #[test]
    fn tests_for_symbol_name_resolves_to_file_first() {
        let mut g = CodeGraph::new();
        g.insert_symbol(sym("processRequest", "src/handler.ts"));
        let hits = tests_for(&g, "processRequest");
        // No importers at all → empty; just verifies it doesn't panic.
        assert!(hits.is_empty());
    }

    #[test]
    fn tests_for_recognises_double_underscore_tests_dir() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        let src_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: "handler".to_string(),
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
        let hits = tests_for(&g, "src/handler.ts");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.to_string_lossy().contains("__tests__"));
    }

    #[test]
    fn tests_for_recognises_spec_suffix() {
        use crate::graph::types::{Confidence, Edge, EdgeKind, SymbolKind};
        let mut g = CodeGraph::new();
        let src_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: "handler".to_string(),
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
        let hits = tests_for(&g, "src/handler.ts");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].file.to_string_lossy().contains(".spec."));
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

        // `find hi` returns the high-centrality hit first (only one match).
        let find_hits = find(&g, "hi", 10);
        assert_eq!(find_hits.len(), 1);
        assert_eq!(find_hits[0].file, PathBuf::from("hi.ts"));

        // `callers of target` orders hi before lo because hi has centrality 100.
        let caller_hits = callers_of(&g, "target", 10);
        assert_eq!(caller_hits[0].file, PathBuf::from("hi.ts"));
        assert_eq!(caller_hits[1].file, PathBuf::from("lo.ts"));
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
        let hits = libraries(&g);
        assert_eq!(hits.len(), 3);
        let lodash_hit = hits
            .iter()
            .find(|h| h.signature.as_deref().is_some_and(|s| s.contains("lodash")))
            .expect("lodash missing");
        assert!(lodash_hit.signature.as_deref().unwrap().contains("2 uses"));
    }
}
