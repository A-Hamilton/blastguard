//! Adapters that render `BlastGuardError` into `rmcp::CallToolResult`
//! with `is_error: true`. Every MCP tool handler returns
//! `Result<Json<T>, CallToolResult>` where the error branch lands here.

use rmcp::model::{CallToolResult, Content};

use crate::error::BlastGuardError;

/// Map any [`BlastGuardError`] to a `CallToolResult` with `is_error: true`.
///
/// The single text content block carries the `Display` representation —
/// error variants already include path, line, similarity, etc. via their
/// `thiserror` `#[error("...")]` attributes.
#[must_use]
pub fn to_error_result(err: &BlastGuardError) -> CallToolResult {
    CallToolResult::error(vec![Content::text(err.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn edit_not_found_rendered_with_closest_match() {
        let err = BlastGuardError::EditNotFound {
            path: PathBuf::from("src/a.ts"),
            line: 5,
            similarity: 0.92,
            fragment: "function processRequest(req) {".to_string(),
        };
        let result = to_error_result(&err);
        assert_eq!(result.is_error, Some(true));
        // Content::text produces a Content::Text variant. We serialise the
        // whole content vec to JSON and search in the string — this avoids
        // any dependency on internal Content field names.
        let serialised = serde_json::to_string(&result.content).unwrap_or_default();
        assert!(
            serialised.contains("src/a.ts"),
            "expected 'src/a.ts' in content; got: {serialised}"
        );
    }

    #[test]
    fn no_test_runner_flags_is_error() {
        let err = BlastGuardError::NoTestRunner;
        let result = to_error_result(&err);
        assert_eq!(result.is_error, Some(true));
    }
}
