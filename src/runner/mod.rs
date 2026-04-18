//! Test runner integration — SPEC §3.3.
//!
//! Auto-detects jest / vitest / pytest / cargo test from project files,
//! runs the chosen command with JSON output, and parses failures into
//! structured records that `mcp::run_tests` annotates with session
//! attribution.

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
