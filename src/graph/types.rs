//! Data structures for the code graph — SPEC.md §7.
//!
//! Wire-format stability: these types are serialised into the on-disk cache
//! via `rmp-serde`. Any breaking change must bump [`CacheFile::version`] in
//! `src/index/cache.rs`; the cache is dropped and rebuilt on mismatch.

use std::collections::HashMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Stable identifier for a symbol, keyed by `(file, name, kind)`.
///
/// Two symbols with the same textual name in the same file are distinct if
/// they have different [`SymbolKind`]s (e.g. a class `Foo` and a type alias
/// `Foo` co-exist in some TS files; both are reachable via the graph).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct SymbolId {
    pub file: PathBuf,
    pub name: String,
    pub kind: SymbolKind,
}

/// Discriminant for symbol lookups. Mirrors the cross-language union of
/// declaration forms we care about for graph queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum SymbolKind {
    Function,
    AsyncFunction,
    Class,
    Method,
    Interface,
    TypeAlias,
    Constant,
    Export,
    Module,
    Trait,
    Struct,
    /// Enum declaration (Rust `enum`, future language equivalents).
    Enum,
}

/// Visibility at the module boundary. Phase 1 treats this as informational;
/// Phase 2 `VISIBILITY` cascade warning consumes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Visibility {
    /// Exported from the module surface (TS `export`, Python `__all__`, Rust `pub`).
    Export,
    /// Accessible within the crate / package but not exported.
    Public,
    /// File- or module-private.
    Private,
    /// Language default (e.g. Python module-level without underscore prefix).
    Default,
}

/// A single declaration extracted from source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub line_start: u32,
    pub line_end: u32,
    pub signature: String,
    pub params: Vec<String>,
    pub return_type: Option<String>,
    pub visibility: Visibility,
    pub body_hash: u64,
    pub is_async: bool,
    /// Row ID in the sqlite-vec table (Phase 2 only).
    #[serde(default)]
    pub embedding_id: Option<i64>,
}

/// Edge classification. Every cascade warning ultimately consults an edge kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    Calls,
    Imports,
    Inherits,
    Implements,
    Exports,
    ReExports,
    TypeReference,
}

/// Confidence in an edge's correctness.
///
/// Dynamic dispatch (Python `getattr`, JS `obj[method]()`), duck typing, and
/// uncertain-but-resolved imports get [`Confidence::Inferred`]. The agent sees
/// these with an explicit caveat in the tool response rather than us dropping
/// them.
///
/// [`Confidence::Unresolved`] is a distinct sentinel for edges whose `to.file`
/// or `to.kind` is still a raw placeholder — not to be confused with "resolved
/// but uncertain". Task 8 (import resolver) and Task 13 (cross-file call
/// resolver) rewrite these fields and upgrade the confidence to `Certain` or
/// `Inferred` based on the resolution outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Confidence {
    Certain,
    Inferred,
    /// Target file or kind is a placeholder — Task 8 / Task 13 resolvers
    /// will rewrite `to.file` / `to.kind` and upgrade to `Certain` or
    /// `Inferred` based on the resolution outcome.
    Unresolved,
}

/// A directed edge between two symbols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub from: SymbolId,
    pub to: SymbolId,
    pub kind: EdgeKind,
    pub line: u32,
    pub confidence: Confidence,
}

/// External (npm/pypi/crates.io) import site. Not a graph edge — library
/// imports exit the graph because we do not index vendored code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryImport {
    pub library: String,
    pub symbol: String,
    pub file: PathBuf,
    pub line: u32,
}

/// In-memory code graph. Both forward and reverse adjacency lists are
/// maintained on every mutation so `search` dispatches are O(degree).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct CodeGraph {
    pub symbols: HashMap<SymbolId, Symbol>,
    pub forward_edges: HashMap<SymbolId, Vec<Edge>>,
    pub reverse_edges: HashMap<SymbolId, Vec<Edge>>,
    pub file_symbols: HashMap<PathBuf, Vec<SymbolId>>,
    pub library_imports: Vec<LibraryImport>,
    /// Reverse-edge in-degree, cached for result ranking.
    pub centrality: HashMap<SymbolId, u32>,
}

impl CodeGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a symbol, indexing it under `file_symbols`.
    pub fn insert_symbol(&mut self, sym: Symbol) {
        self.file_symbols
            .entry(sym.id.file.clone())
            .or_default()
            .push(sym.id.clone());
        self.symbols.insert(sym.id.clone(), sym);
    }

    /// Insert an edge, updating forward, reverse, and centrality in lockstep.
    pub fn insert_edge(&mut self, edge: Edge) {
        let to = edge.to.clone();
        let from = edge.from.clone();
        self.forward_edges
            .entry(from)
            .or_default()
            .push(edge.clone());
        self.reverse_edges
            .entry(to.clone())
            .or_default()
            .push(edge);
        *self.centrality.entry(to).or_insert(0) += 1;
    }

    /// Remove every symbol and edge belonging to `file`. Used by the file
    /// watcher when a file is deleted and by the incremental reindexer before
    /// re-parsing a changed file.
    pub fn remove_file(&mut self, file: &std::path::Path) {
        let Some(symbol_ids) = self.file_symbols.remove(file) else {
            return;
        };
        for id in &symbol_ids {
            self.symbols.remove(id);
            // Forward edges originating from this symbol.
            if let Some(outgoing) = self.forward_edges.remove(id) {
                for edge in outgoing {
                    if let Some(rev) = self.reverse_edges.get_mut(&edge.to) {
                        rev.retain(|e| e.from != edge.from);
                    }
                    if let Some(c) = self.centrality.get_mut(&edge.to) {
                        *c = c.saturating_sub(1);
                    }
                }
            }
            // Drop the reverse index entry but keep callers' forward edges dangling.
            // detect_orphan (Phase 1.6) iterates those dangling edges to find
            // callers of removed symbols; pruning them here would silently hide
            // cascade failures.
            self.reverse_edges.remove(id);
            self.centrality.remove(id);
        }
        self.library_imports.retain(|li| li.file != file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 3,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    fn edge(from: &Symbol, to: &Symbol) -> Edge {
        Edge {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Calls,
            line: 2,
            confidence: Confidence::Certain,
        }
    }

    #[test]
    fn insert_symbol_indexes_by_file() {
        let mut g = CodeGraph::new();
        let a = sym("a", "x.ts");
        g.insert_symbol(a.clone());
        assert_eq!(g.file_symbols.get(&a.id.file).map(Vec::len), Some(1));
        assert!(g.symbols.contains_key(&a.id));
    }

    #[test]
    fn insert_edge_updates_both_sides() {
        let mut g = CodeGraph::new();
        let a = sym("a", "x.ts");
        let b = sym("b", "x.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        g.insert_edge(edge(&a, &b));
        assert_eq!(g.forward_edges.get(&a.id).map(Vec::len), Some(1));
        assert_eq!(g.reverse_edges.get(&b.id).map(Vec::len), Some(1));
        assert_eq!(g.centrality.get(&b.id).copied(), Some(1));
    }

    #[test]
    fn remove_file_preserves_caller_forward_edges_when_callee_removed() {
        // When the callee's file is deleted, the caller's forward edge to it must
        // remain — that dangling edge is how Phase 1.6's detect_orphan finds
        // broken calls. Regression guard: fixing remove_file broke this on ccb9d49.
        let mut g = CodeGraph::new();
        let src = sym("a", "x.ts");
        let dst = sym("b", "y.ts");
        g.insert_symbol(src.clone());
        g.insert_symbol(dst.clone());
        g.insert_edge(edge(&src, &dst));

        g.remove_file(std::path::Path::new("y.ts"));

        // Callee is gone, reverse index for it is gone, but caller's forward edge
        // to the (now-missing) callee is retained.
        assert!(!g.symbols.contains_key(&dst.id));
        assert!(!g.reverse_edges.contains_key(&dst.id));
        let src_fwd = g.forward_edges.get(&src.id).expect("caller forward edges");
        assert_eq!(src_fwd.len(), 1, "caller's forward edge to deleted callee must be kept");
        assert_eq!(src_fwd[0].to, dst.id);
    }

    #[test]
    fn remove_file_drops_symbols_and_edges() {
        let mut g = CodeGraph::new();
        let a = sym("a", "x.ts");
        let b = sym("b", "y.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        g.insert_edge(edge(&a, &b));
        g.remove_file(std::path::Path::new("x.ts"));
        assert!(!g.symbols.contains_key(&a.id));
        assert!(g.symbols.contains_key(&b.id));
        assert!(!g.forward_edges.contains_key(&a.id));
        // The edge from a -> b is pruned from b's reverse list.
        assert!(g
            .reverse_edges
            .get(&b.id)
            .is_none_or(|v| v.iter().all(|e| e.from != a.id)));
    }
}
