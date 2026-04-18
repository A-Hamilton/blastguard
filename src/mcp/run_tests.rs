//! `run_tests` MCP tool handler.
//!
//! Plan 5's rmcp `#[tool]` macro will wrap this. For now it's a simple
//! pass-through that Plan 5's adapter can call directly.

use std::path::Path;
use std::sync::Mutex;

use crate::error::Result;
use crate::graph::types::CodeGraph;
use crate::runner::{run_tests, RunTestsRequest, RunTestsResponse};
use crate::session::SessionState;

/// Thin pass-through to [`crate::runner::run_tests`].
///
/// # Errors
/// Bubbles any error from the orchestrator; Plan 5's `#[tool]` adapter
/// maps them to `CallToolResult { is_error: true, .. }`.
pub fn handle(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: &RunTestsRequest,
) -> Result<RunTestsResponse> {
    run_tests(graph, session, project_root, request)
}
