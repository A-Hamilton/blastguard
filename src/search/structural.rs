//! Graph-backed search backends — SPEC §3.1.
//!
//! Each public function resolves a [`super::query::QueryKind`] arm against the
//! [`CodeGraph`] and renders hits via [`super::hit::SearchHit::structural`].

use crate::graph::ops::{callers, find_by_name};
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
}
