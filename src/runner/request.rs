//! Request and response DTOs for the `run_tests` tool.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::TestResults;

/// Input to the `run_tests` MCP tool. Mirrors SPEC §3.3.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct RunTestsRequest {
    /// Optional test-name filter passed to the runner (jest `-t`, pytest `-k`, etc.).
    /// When `None`, runs the whole suite.
    #[serde(default)]
    pub filter: Option<String>,
    /// Kill the runner if it exceeds this wall-clock budget. Defaults to 60s.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

const fn default_timeout_seconds() -> u64 {
    60
}

/// Response body for a completed `run_tests` invocation. `failures` carries
/// `YOU MODIFIED X (N edits ago)` suffixes applied by
/// [`super::attribute::annotate_failures`] (Task 9).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RunTestsResponse {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration_ms: u64,
    pub failures: Vec<String>,
}

impl From<TestResults> for RunTestsResponse {
    fn from(r: TestResults) -> Self {
        Self {
            passed: r.passed,
            failed: r.failed,
            skipped: r.skipped,
            duration_ms: r.duration_ms,
            failures: r
                .failures
                .into_iter()
                .map(|f| {
                    format!(
                        "FAIL {}:{} {} \u{2014} {}",
                        f.file.display(),
                        f.line,
                        f.test_name,
                        f.message
                    )
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_applied() {
        let req: RunTestsRequest = serde_json::from_str("{}").expect("parse");
        assert!(req.filter.is_none());
        assert_eq!(req.timeout_seconds, 60);
    }

    #[test]
    fn request_accepts_filter_and_timeout() {
        let req: RunTestsRequest =
            serde_json::from_str(r#"{"filter": "auth", "timeout_seconds": 30}"#)
                .expect("parse");
        assert_eq!(req.filter.as_deref(), Some("auth"));
        assert_eq!(req.timeout_seconds, 30);
    }

    #[test]
    fn response_from_results_renders_fail_lines() {
        use crate::runner::TestFailure;
        use std::path::PathBuf;

        let results = TestResults {
            passed: 10,
            failed: 1,
            skipped: 0,
            duration_ms: 250,
            failures: vec![TestFailure {
                test_name: "test_foo".to_string(),
                file: PathBuf::from("tests/a.rs"),
                line: 42,
                message: "assertion failed".to_string(),
                stack: vec![],
            }],
        };
        let resp: RunTestsResponse = results.into();
        assert_eq!(resp.passed, 10);
        assert_eq!(resp.failures.len(), 1);
        assert!(resp.failures[0].starts_with("FAIL tests/a.rs:42"));
    }
}
