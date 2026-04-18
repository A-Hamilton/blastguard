//! Graph-backed search backends — SPEC §3.1.
//!
//! Each public function resolves a [`super::query::QueryKind`] arm against the
//! [`CodeGraph`] and renders hits via [`super::hit::SearchHit::structural`].

use crate::graph::ops::{callees, callers, find_by_name, shortest_path};
use crate::graph::types::{CodeGraph, SymbolId};
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
}
