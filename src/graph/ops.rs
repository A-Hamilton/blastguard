//! Graph traversal helpers: callers, callees, shortest path, outline.
//!
//! Phase 1.1 lands the BFS primitives used by every `search` dispatcher
//! pattern (SPEC §3.1) and by cascade impact analysis (SPEC §5).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::graph::types::{CodeGraph, SymbolId};

/// Return direct callers of `target` (reverse-edge lookup). Cheap: O(degree).
#[must_use]
pub fn callers<'g>(graph: &'g CodeGraph, target: &SymbolId) -> Vec<&'g SymbolId> {
    graph
        .reverse_edges
        .get(target)
        .map(|edges| edges.iter().map(|e| &e.from).collect())
        .unwrap_or_default()
}

/// Return direct callees of `target` (forward-edge lookup).
#[must_use]
pub fn callees<'g>(graph: &'g CodeGraph, target: &SymbolId) -> Vec<&'g SymbolId> {
    graph
        .forward_edges
        .get(target)
        .map(|edges| edges.iter().map(|e| &e.to).collect())
        .unwrap_or_default()
}

/// BFS shortest path from `from` to any node matching `pred`, following
/// forward edges. `None` when unreachable. Returns the chain of symbols in
/// order, terminating at the first node for which `pred(id)` is true.
#[must_use]
pub fn shortest_path_to_predicate<F>(
    graph: &CodeGraph,
    from: &SymbolId,
    pred: F,
) -> Option<Vec<SymbolId>>
where
    F: Fn(&SymbolId) -> bool,
{
    if pred(from) {
        return Some(vec![from.clone()]);
    }
    let mut queue: VecDeque<SymbolId> = VecDeque::new();
    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut parent: HashMap<SymbolId, SymbolId> = HashMap::new();
    queue.push_back(from.clone());
    visited.insert(from.clone());

    while let Some(current) = queue.pop_front() {
        let Some(edges) = graph.forward_edges.get(&current) else {
            continue;
        };
        for edge in edges {
            if visited.insert(edge.to.clone()) {
                parent.insert(edge.to.clone(), current.clone());
                if pred(&edge.to) {
                    let mut path = vec![edge.to.clone()];
                    let mut node = edge.to.clone();
                    while let Some(p) = parent.get(&node) {
                        path.push(p.clone());
                        if p == from {
                            break;
                        }
                        node = p.clone();
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(edge.to.clone());
            }
        }
    }
    None
}

/// BFS shortest path from `from` to `to`, following forward edges. `None`
/// when unreachable. Returns the chain of symbols in order.
#[must_use]
pub fn shortest_path(graph: &CodeGraph, from: &SymbolId, to: &SymbolId) -> Option<Vec<SymbolId>> {
    shortest_path_to_predicate(graph, from, |id| id == to)
}

/// Centrality-sorted list of symbols matching `name` (exact match first,
/// Levenshtein ≤ 2 second). Used by the `find X` dispatcher.
#[must_use]
pub fn find_by_name<'g>(graph: &'g CodeGraph, name: &str) -> Vec<&'g SymbolId> {
    let mut exact: Vec<&SymbolId> = graph.symbols.keys().filter(|id| id.name == name).collect();
    exact.sort_by_key(|id| std::cmp::Reverse(graph.centrality.get(*id).copied().unwrap_or(0)));

    if !exact.is_empty() {
        return exact;
    }

    let mut fuzzy: Vec<(&SymbolId, usize)> = graph
        .symbols
        .keys()
        .filter_map(|id| {
            let d = strsim::levenshtein(&id.name, name);
            if d <= 2 {
                Some((id, d))
            } else {
                None
            }
        })
        .collect();
    fuzzy.sort_by(|a, b| {
        a.1.cmp(&b.1).then_with(|| {
            let ca = graph.centrality.get(a.0).copied().unwrap_or(0);
            let cb = graph.centrality.get(b.0).copied().unwrap_or(0);
            cb.cmp(&ca)
        })
    });
    fuzzy.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Confidence, Edge, EdgeKind, Symbol, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn mk(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 2,
            signature: name.to_string(),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    fn connect(g: &mut CodeGraph, from: &Symbol, to: &Symbol) {
        g.insert_edge(Edge {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
    }

    #[test]
    fn callers_returns_reverse_edges() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        connect(&mut g, &a, &b);
        assert_eq!(callers(&g, &b.id), vec![&a.id]);
        assert!(callers(&g, &a.id).is_empty());
    }

    #[test]
    fn shortest_path_walks_forward_edges() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        let c = mk("c", "x.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        g.insert_symbol(c.clone());
        connect(&mut g, &a, &b);
        connect(&mut g, &b, &c);
        let path = shortest_path(&g, &a.id, &c.id).expect("reachable");
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], a.id);
        assert_eq!(path[2], c.id);
    }

    #[test]
    fn shortest_path_none_when_unreachable() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        assert!(shortest_path(&g, &a.id, &b.id).is_none());
    }

    #[test]
    fn find_by_name_exact_beats_fuzzy() {
        let mut g = CodeGraph::new();
        let exact = mk("process", "a.ts");
        let near = mk("proces", "b.ts");
        g.insert_symbol(exact.clone());
        g.insert_symbol(near.clone());
        let hits = find_by_name(&g, "process");
        assert_eq!(hits, vec![&exact.id]);
    }

    #[test]
    fn find_by_name_fuzzy_when_no_exact() {
        let mut g = CodeGraph::new();
        let near = mk("proces", "b.ts");
        g.insert_symbol(near.clone());
        let hits = find_by_name(&g, "process");
        assert_eq!(hits, vec![&near.id]);
    }

    #[test]
    fn shortest_path_to_predicate_walks_forward_edges() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        let c = mk("c", "y.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        g.insert_symbol(c.clone());
        connect(&mut g, &a, &b);
        connect(&mut g, &b, &c);
        let path =
            shortest_path_to_predicate(&g, &a.id, |id| id.file == std::path::Path::new("y.ts"))
                .expect("reachable");
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], a.id);
        assert_eq!(path[2], c.id);
    }

    #[test]
    fn shortest_path_to_predicate_none_when_no_node_matches() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        connect(&mut g, &a, &b);
        assert!(shortest_path_to_predicate(&g, &a.id, |_| false).is_none());
    }

    #[test]
    fn shortest_path_to_predicate_short_circuits_when_from_matches() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        g.insert_symbol(a.clone());
        let path = shortest_path_to_predicate(&g, &a.id, |id| id == &a.id)
            .expect("from matches predicate");
        assert_eq!(path, vec![a.id.clone()]);
    }
}
