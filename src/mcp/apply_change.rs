//! `apply_change` MCP tool handler.
//!
//! Plan 4 will wrap this in an rmcp `#[tool]` macro on the server struct.
//! For now the module exports a simple function that Plan 4's adapter
//! can call directly.

use std::path::Path;
use std::sync::Mutex;

use crate::edit::{apply_change, ApplyChangeRequest, ApplyChangeResponse};
use crate::error::Result;
use crate::graph::types::CodeGraph;
use crate::session::SessionState;

/// Thin pass-through to [`crate::edit::apply_change`].
///
/// # Errors
/// Bubbles any error from the orchestrator; Plan 4's `#[tool]` adapter
/// maps them into `CallToolResult { is_error: true, .. }`.
pub fn handle(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: &ApplyChangeRequest,
) -> Result<ApplyChangeResponse> {
    apply_change(graph, session, project_root, request)
}
