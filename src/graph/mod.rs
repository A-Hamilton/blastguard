//! Code graph: symbols, edges, indices, and operations.
//!
//! The graph is the foundation of every `search` dispatch and every
//! `apply_change` cascade warning. See `SPEC.md` §7 for the data schema
//! and §3.1 for the query patterns it supports.

pub mod impact;
pub mod ops;
pub mod types;

pub use types::{
    CodeGraph, Confidence, Edge, EdgeKind, LibraryImport, Symbol, SymbolId, SymbolKind, Visibility,
};
