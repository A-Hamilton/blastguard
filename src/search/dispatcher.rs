//! Top-level search dispatcher — routes query strings to structural or text
//! backends. Implemented arm-by-arm starting in Plan 2 Task 3.

use std::path::Path;

use crate::graph::types::CodeGraph;

use super::SearchHit;

/// Dispatch a query. Arms are filled in sequentially from Task 3 onwards.
#[must_use]
pub fn dispatch(_graph: &CodeGraph, _project_root: &Path, _query: &str) -> Vec<SearchHit> {
    Vec::new()
}
