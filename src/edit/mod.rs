//! `apply_change` tool backend — SPEC §3.2.
//!
//! Orchestrates: (1) disk edit via [`apply`], (2) reparse via
//! [`crate::parse`], (3) symbol diff via [`diff`], (4) cascade detection
//! via [`crate::graph::impact`], (5) bundled context via [`context`].
//!
//! Plan 4 wires the entry point into an rmcp `#[tool]` handler. For now
//! the orchestrator in [`apply::orchestrate`] returns a plain [`Result`]
//! that the caller can map into `CallToolResult { is_error: true, .. }`
//! on failure.

pub mod apply;
pub mod context;
pub mod diff;
pub mod request;

pub use request::{ApplyChangeRequest, ApplyChangeResponse, ApplyStatus, BundledContext, Change};

use std::path::Path;
use std::sync::Mutex;

use crate::error::Result;
use crate::graph::types::CodeGraph;
use crate::session::SessionState;

/// Entry point for the `apply_change` tool backend.
///
/// Orchestrates: apply the edit(s) to disk → reparse the file → diff the
/// symbol table → run cascade detectors → build bundled context → record
/// into [`SessionState`]. Graph and session are `&Mutex<...>` so Plan 4's
/// rmcp handler can thread shared state in.
///
/// # Errors
/// Bubbles any error from disk I/O, edit resolution, or parse failure.
/// Plan 4's MCP adapter maps them to `CallToolResult { is_error: true, .. }`.
///
/// # Panics
/// Panics only if the `graph` or `session` mutex has been poisoned by a
/// previous thread panic, which is a fatal condition in this server.
pub fn apply_change(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: &ApplyChangeRequest,
) -> Result<ApplyChangeResponse> {
    apply::orchestrate(graph, session, project_root, request)
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use crate::graph::impact::WarningKind;
    use crate::index::indexer::cold_index;

    #[test]
    fn signature_edit_fires_signature_warning() {
        // Phase 1 only emits intra-file Calls edges (cross-file call resolution
        // is Task 13). Place both the callee and caller in the same file so
        // cold_index emits the edge that detect_signature can find.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .expect("git init");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        // One file: `processRequest` is defined and called by `api` in the same
        // file, so tree-sitter emits a Calls edge from api → processRequest.
        std::fs::write(
            tmp.path().join("src/handler.ts"),
            concat!(
                "export function processRequest(req) { return req; }\n",
                "export function api() { return processRequest({}); }\n",
            ),
        )
        .expect("write handler");

        let graph = Mutex::new(cold_index(tmp.path()).expect("cold_index"));
        let session = Mutex::new(SessionState::new());

        let req = ApplyChangeRequest {
            file: tmp.path().join("src/handler.ts"),
            changes: vec![Change {
                old_text: "processRequest(req)".to_string(),
                new_text: "processRequest(req, res)".to_string(),
            }],
            create_file: false,
            delete_file: false,
        };

        let resp = apply_change(&graph, &session, tmp.path(), &req).expect("apply");
        assert_eq!(resp.status, ApplyStatus::Applied);
        assert!(
            resp.warnings.iter().any(|w| w.kind == WarningKind::Signature),
            "expected SIGNATURE; got {:?}",
            resp.warnings
        );
        // Caller `api` is in the same file — verify context sees it.
        assert!(
            resp.context.callers.iter().any(|c| c.contains("handler.ts")),
            "expected handler.ts in callers; got {:?}",
            resp.context.callers
        );
    }

    #[test]
    fn apply_change_returns_edit_not_found_error_for_bogus_old_text() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(tmp.path().join("src/a.ts"), "export function foo() {}\n")
            .expect("write");

        let graph = Mutex::new(CodeGraph::new());
        let session = Mutex::new(SessionState::new());
        let req = ApplyChangeRequest {
            file: tmp.path().join("src/a.ts"),
            changes: vec![Change {
                old_text: "THIS_TEXT_DOES_NOT_EXIST".to_string(),
                new_text: "x".to_string(),
            }],
            create_file: false,
            delete_file: false,
        };
        let err = apply_change(&graph, &session, tmp.path(), &req).expect_err("should fail");
        assert!(
            matches!(err, crate::error::BlastGuardError::EditNotFound { .. }),
            "got {err:?}"
        );
    }
}
