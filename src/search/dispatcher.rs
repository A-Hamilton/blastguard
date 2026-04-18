//! Query classifier for `search` — SPEC §3.1 dispatcher table.
//!
//! Phase 1.5 lands the full regex ladder. This module holds the placeholder
//! types and classifier entry point so the rest of the crate compiles.

use serde::Serialize;

use crate::graph::types::CodeGraph;

/// A single search result. Rendered to an MCP text block by the tool handler.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub file: std::path::PathBuf,
    pub line: u32,
    pub signature: String,
    pub snippet: Option<String>,
}

/// Dispatch a query. Phase 1.5 will route to structural or grep based on
/// pattern recognition; for now it returns an empty result set.
#[must_use]
pub fn dispatch(_graph: &CodeGraph, _query: &str) -> Vec<SearchHit> {
    // TODO(phase-1.5): regex ladder per SPEC §3.1 dispatch table.
    Vec::new()
}
