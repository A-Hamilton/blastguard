//! Test runner integration — SPEC §3.3.
//!
//! Auto-detects jest / vitest / pytest / cargo test from project files,
//! runs the chosen command with JSON output, and parses failures into
//! structured records that `mcp::run_tests` annotates with session
//! attribution.

pub mod attribute;
pub mod detect;
pub mod execute;
pub mod parse;
pub mod request;

use serde::{Deserialize, Serialize};

/// Single failing test case extracted from the runner output. Raw —
/// `mcp::run_tests` adds the `YOU MODIFIED X` suffix before sending it over
/// the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFailure {
    pub test_name: String,
    pub file: std::path::PathBuf,
    pub line: u32,
    pub message: String,
    /// `<File:line>` pairs lifted from the stack trace, in call order.
    pub stack: Vec<(std::path::PathBuf, u32)>,
}

/// Aggregate result of a single `run_tests` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration_ms: u64,
    pub failures: Vec<TestFailure>,
}

/// Enum of detected runners. `detect::autodetect` maps project files to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Runner {
    Jest,
    Vitest,
    Pytest,
    CargoTest,
}

pub use request::{RunTestsRequest, RunTestsResponse};

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use crate::error::{BlastGuardError, Result};
use crate::graph::types::CodeGraph;
use crate::session::SessionState;

/// Entry point for the `run_tests` tool backend.
///
/// Sequence: [`detect::autodetect`] → [`execute::build_command`] →
/// [`execute::run`] with timeout → [`parse::parse`] →
/// [`attribute::annotate_failures`] → [`SessionState::record_test_results`]
/// → [`RunTestsResponse`] (via `From<TestResults>`).
///
/// # Errors
/// - [`BlastGuardError::NoTestRunner`] when detection returns `None`.
/// - [`BlastGuardError::TestTimeout`] when the runner exceeds its budget.
/// - [`BlastGuardError::TestCrashed`] when spawn/wait fails. Normal
///   failing-tests return `Ok` with `failed > 0`.
///
/// # Panics
/// Panics if the `graph` or `session` `Mutex` is poisoned (a previous thread
/// panicked while holding the lock — the process is in an unrecoverable state
/// at that point and a panic is appropriate).
pub fn run_tests(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: &RunTestsRequest,
) -> Result<RunTestsResponse> {
    let runner = detect::autodetect(project_root).ok_or(BlastGuardError::NoTestRunner)?;
    let cmd = execute::build_command(runner, project_root, request.filter.as_deref());
    let exec = execute::run(cmd, Duration::from_secs(request.timeout_seconds))?;

    if exec.timed_out {
        return Err(BlastGuardError::TestTimeout {
            seconds: request.timeout_seconds,
        });
    }

    let mut results = parse::parse(runner, &exec.stdout);
    results.duration_ms = u64::try_from(exec.duration.as_millis()).unwrap_or(u64::MAX);

    let annotated = {
        let g = graph.lock().expect("graph lock poisoned");
        let s = session.lock().expect("session lock poisoned");
        attribute::annotate_failures(&g, &s, results.failures)
    };
    results.failures = annotated;

    {
        let mut s = session.lock().expect("session lock poisoned");
        s.record_test_results(results.clone());
    }

    Ok(results.into())
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use crate::graph::types::CodeGraph;
    use std::sync::Mutex;

    #[test]
    fn no_test_runner_error_when_project_has_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let graph = Mutex::new(CodeGraph::new());
        let session = Mutex::new(SessionState::new());
        let req = RunTestsRequest::default();
        let err = run_tests(&graph, &session, tmp.path(), &req).expect_err("should error");
        assert!(matches!(err, BlastGuardError::NoTestRunner), "got {err:?}");
    }
}
