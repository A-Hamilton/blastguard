//! Search result record and formatting helpers.

use std::cmp::Reverse;
use std::path::PathBuf;

use serde::Serialize;

use crate::graph::types::{CodeGraph, Symbol, SymbolId};

/// A single search result. Rendered to an MCP text block by the tool handler.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchHit {
    pub file: PathBuf,
    pub line: u32,
    /// Inline signature for structural results (e.g. `processRequest(req: Request): Promise<Response>`).
    /// `None` for grep hits.
    pub signature: Option<String>,
    /// Raw matching line for grep hits. `None` for structural hits.
    pub snippet: Option<String>,
}

impl SearchHit {
    /// Build a structural hit from a parsed symbol. Copies the signature
    /// through so the MCP response renders inline without a follow-up read.
    #[must_use]
    pub fn structural(symbol: &Symbol) -> Self {
        Self {
            file: symbol.id.file.clone(),
            line: symbol.line_start,
            signature: Some(symbol.signature.clone()),
            snippet: None,
        }
    }

    /// Build a grep hit from a raw `file:line` match.
    #[must_use]
    pub fn grep(file: PathBuf, line: u32, snippet: String) -> Self {
        Self {
            file,
            line,
            signature: None,
            snippet: Some(snippet),
        }
    }
}

/// Sort a slice of [`SymbolId`] references by reverse-edge centrality descending.
///
/// Used to rank multiple matches in `find_by_name` / `callers_of` so the
/// highest-dependent symbols come first.
pub fn sort_by_centrality(graph: &CodeGraph, ids: &mut [&SymbolId]) {
    ids.sort_by_key(|id| Reverse(graph.centrality.get(*id).copied().unwrap_or(0)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};

    fn sym(name: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from("x.ts"),
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
    fn structural_hit_copies_signature() {
        let s = sym("foo");
        let hit = SearchHit::structural(&s);
        assert_eq!(hit.signature.as_deref(), Some("fn foo()"));
        assert!(hit.snippet.is_none());
        assert_eq!(hit.file, PathBuf::from("x.ts"));
        assert_eq!(hit.line, 1);
    }

    #[test]
    fn grep_hit_carries_snippet_only() {
        let hit = SearchHit::grep(PathBuf::from("a.ts"), 5, "  const x = foo();".to_string());
        assert!(hit.signature.is_none());
        assert_eq!(hit.snippet.as_deref(), Some("  const x = foo();"));
    }

    #[test]
    fn sort_by_centrality_orders_highest_first() {
        let mut g = CodeGraph::new();
        let low = sym("low");
        let high = sym("high");
        g.insert_symbol(low.clone());
        g.insert_symbol(high.clone());
        g.centrality.insert(low.id.clone(), 1);
        g.centrality.insert(high.id.clone(), 10);
        let mut ids = vec![&low.id, &high.id];
        sort_by_centrality(&g, &mut ids);
        assert_eq!(ids[0], &high.id);
        assert_eq!(ids[1], &low.id);
    }

    #[test]
    fn sort_by_centrality_missing_entries_treated_as_zero() {
        let mut g = CodeGraph::new();
        let only_in_centrality = sym("a");
        let not_in_centrality = sym("b");
        g.insert_symbol(only_in_centrality.clone());
        g.insert_symbol(not_in_centrality.clone());
        g.centrality.insert(only_in_centrality.id.clone(), 5);
        let mut ids = vec![&not_in_centrality.id, &only_in_centrality.id];
        sort_by_centrality(&g, &mut ids);
        // The one with centrality=5 must come before the one missing (treated as 0).
        assert_eq!(ids[0], &only_in_centrality.id);
    }
}
