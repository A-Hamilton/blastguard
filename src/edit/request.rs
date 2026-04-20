//! Request and response DTOs for the `apply_change` tool.
//!
//! Derives `Serialize`/`Deserialize`/`JsonSchema` so Plan 4's rmcp
//! `#[tool]` handler can round-trip them over the MCP wire without a
//! bridging layer.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::impact::Warning;

/// One text replacement inside a single `apply_change` call. `old_text`
/// must appear exactly once in `file`; zero or multiple matches surface
/// as [`crate::error::BlastGuardError`] variants.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Change {
    /// The exact text to find in the file. Must appear exactly once.
    pub old_text: String,
    /// The replacement text.
    pub new_text: String,
}

/// Input to the `apply_change` MCP tool. Mirrors SPEC §3.2 exactly.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ApplyChangeRequest {
    /// Path to the file to edit, relative to the project root.
    pub file: PathBuf,
    /// Ordered list of text replacements to apply to `file`.
    #[serde(default)]
    pub changes: Vec<Change>,
    /// When `true`, create the file at `file` (must not already exist).
    #[serde(default)]
    pub create_file: bool,
    /// When `true`, delete the file at `file` (must exist).
    #[serde(default)]
    pub delete_file: bool,
}

/// Response body for a successful `apply_change` invocation. Error paths
/// surface as [`crate::error::BlastGuardError`] from the orchestrator and
/// are mapped to `CallToolResult { is_error: true, .. }` by Plan 4's MCP
/// adapter.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ApplyChangeResponse {
    /// Outcome category of this operation.
    pub status: ApplyStatus,
    /// One-line summary, e.g. `"Modified processRequest() in src/handler.ts.
    /// 2 warnings: 1 SIGNATURE, 1 ASYNC_CHANGE."`.
    pub summary: String,
    /// Cascade warnings detected for each changed symbol.
    pub warnings: Vec<Warning>,
    /// Pre-fetched follow-up context bundled into the response.
    pub context: BundledContext,
    /// Per-change minimal unified diff, e.g.
    /// `"@@ src/a.ts:L12 @@\n-foo\n+bar\n"`. Empty on `Deleted` / `Created`
    /// status (the summary already names the file). Lets the agent confirm
    /// the edit landed without re-reading the file.
    #[serde(default)]
    pub diff_snippet: String,
}

/// Top-level status for an applied change. Serialised as `SCREAMING_SNAKE_CASE`
/// to match the MCP wire convention already used by `WarningKind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApplyStatus {
    /// A normal in-place edit landed.
    Applied,
    /// A new file was created via `create_file: true`.
    Created,
    /// A file was deleted via `delete_file: true`.
    Deleted,
    /// No material change (whitespace/comments-only edit).
    NoOp,
}

/// Pre-fetched follow-up data so the agent rarely needs another search.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct BundledContext {
    /// Inline caller snippets for each changed symbol — `"file:line — signature"`.
    pub callers: Vec<String>,
    /// Test files importing the edited file.
    pub tests: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_status_serialises_as_screaming_snake() {
        assert_eq!(
            serde_json::to_string(&ApplyStatus::Applied).unwrap(),
            "\"APPLIED\""
        );
        assert_eq!(
            serde_json::to_string(&ApplyStatus::NoOp).unwrap(),
            "\"NO_OP\""
        );
        assert_eq!(
            serde_json::to_string(&ApplyStatus::Created).unwrap(),
            "\"CREATED\""
        );
        assert_eq!(
            serde_json::to_string(&ApplyStatus::Deleted).unwrap(),
            "\"DELETED\""
        );
    }

    #[test]
    fn request_round_trips_with_defaults() {
        let json = r#"{"file": "src/a.ts", "changes": []}"#;
        let req: ApplyChangeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.file, PathBuf::from("src/a.ts"));
        assert!(req.changes.is_empty());
        assert!(!req.create_file);
        assert!(!req.delete_file);
    }

    #[test]
    fn request_accepts_multiple_changes() {
        let json = r#"{
            "file": "src/a.ts",
            "changes": [
                {"old_text": "foo", "new_text": "bar"},
                {"old_text": "baz", "new_text": "qux"}
            ]
        }"#;
        let req: ApplyChangeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.changes.len(), 2);
        assert_eq!(req.changes[0].old_text, "foo");
        assert_eq!(req.changes[1].new_text, "qux");
    }
}
