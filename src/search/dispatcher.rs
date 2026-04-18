//! Top-level search dispatcher — classifies a query string and routes it to
//! the structural or text backend. Arms are wired incrementally as each
//! backend function lands.

use std::path::Path;

use crate::graph::types::CodeGraph;

use super::query::{classify, QueryKind};
use super::{structural, SearchHit};

/// Default cap for structural results. Matches the token budget in SPEC §3
/// for list-style queries (50-150 tokens for callers/callees; keeping 10
/// hits leaves headroom for inline signatures).
const DEFAULT_MAX_HITS: usize = 10;

/// Classify and route a query. Returns an empty `Vec` when no backend arm
/// has been wired for the matched `QueryKind` yet — remaining arms land in
/// Tasks 4-12.
#[must_use]
pub fn dispatch(graph: &CodeGraph, _project_root: &Path, query: &str) -> Vec<SearchHit> {
    match classify(query) {
        QueryKind::Find(name) => structural::find(graph, &name, DEFAULT_MAX_HITS),
        QueryKind::Callers(name) => structural::callers_of(graph, &name, DEFAULT_MAX_HITS),
        QueryKind::Callees(name) => structural::callees_of(graph, &name, DEFAULT_MAX_HITS),
        QueryKind::Outline(path) => structural::outline_of(graph, &path),
        QueryKind::Chain(from, to) => structural::chain_from_to(graph, &from, &to),
        QueryKind::ImportsOf(path) => structural::imports_of(graph, &path),
        QueryKind::ImportersOf(path) => structural::importers_of(graph, &path),
        QueryKind::ExportsOf(path) => structural::exports_of(graph, &path),
        QueryKind::TestsFor(target) => structural::tests_for(graph, &target),
        // Arms below land in subsequent tasks.
        QueryKind::Libraries
        | QueryKind::Grep(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Symbol, SymbolId, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn sym(name: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from("a.ts"),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 2,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    #[test]
    fn dispatches_find_query_to_structural_find() {
        let mut g = CodeGraph::new();
        g.insert_symbol(sym("processRequest"));
        let hits = dispatch(&g, Path::new("."), "find processRequest");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].signature.as_deref(), Some("fn processRequest()"));
    }

    #[test]
    fn dispatches_callers_query() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let t = sym("target");
        let c = sym("caller");
        let t_id = t.id.clone();
        let c_id = c.id.clone();
        g.insert_symbol(t);
        g.insert_symbol(c);
        g.insert_edge(Edge {
            from: c_id,
            to: t_id,
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = dispatch(&g, Path::new("."), "callers of target");
        assert_eq!(hits.len(), 1);
    }
}
