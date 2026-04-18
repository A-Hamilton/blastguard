//! Runner-output parsers: jest, vitest, pytest, cargo-test.
//!
//! Each parser reads the raw stdout of its runner (JSON for jest/vitest/
//! pytest, line-delimited JSON for cargo) and emits a [`TestResults`]
//! with `passed/failed/skipped/duration_ms` counts plus a
//! [`TestFailure`] per failure including `file:line` and stack-trace
//! `file:line` pairs.

use std::path::PathBuf;

use serde_json::Value;

use super::{Runner, TestFailure, TestResults};

/// Dispatch to the correct parser based on the detected [`Runner`].
#[must_use]
pub fn parse(runner: Runner, stdout: &[u8]) -> TestResults {
    match runner {
        Runner::Jest => parse_jest(stdout),
        Runner::Vitest => parse_vitest(stdout),
        Runner::Pytest => parse_pytest(stdout),
        Runner::CargoTest => parse_cargo(stdout),
    }
}

/// Parse jest's `--json` report (single JSON object on stdout).
#[must_use]
fn parse_jest(stdout: &[u8]) -> TestResults {
    let v: Value = serde_json::from_slice(stdout).unwrap_or(Value::Null);
    let passed = v.get("numPassedTests").and_then(Value::as_u64).unwrap_or(0);
    let failed = v.get("numFailedTests").and_then(Value::as_u64).unwrap_or(0);
    let skipped = v.get("numPendingTests").and_then(Value::as_u64).unwrap_or(0);

    let mut failures = Vec::new();
    if let Some(test_results) = v.get("testResults").and_then(Value::as_array) {
        for file in test_results {
            let file_path = file
                .get("name")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_default();
            let assertions = file
                .get("assertionResults")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for a in assertions {
                if a.get("status").and_then(Value::as_str) != Some("failed") {
                    continue;
                }
                let test_name = a
                    .get("fullName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let line = a
                    .get("location")
                    .and_then(|loc| loc.get("line"))
                    .and_then(Value::as_u64)
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0);
                let messages = a
                    .get("failureMessages")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let raw_message = messages
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let stack = parse_stack(&raw_message);
                let message = first_line(&raw_message);

                failures.push(TestFailure {
                    test_name,
                    file: file_path.clone(),
                    line,
                    message,
                    stack,
                });
            }
        }
    }

    TestResults {
        passed: cast_u32(passed),
        failed: cast_u32(failed),
        skipped: cast_u32(skipped),
        duration_ms: 0,
        failures,
    }
}

#[must_use]
fn parse_vitest(stdout: &[u8]) -> TestResults {
    // Vitest --reporter=json matches jest shape closely. Reuse jest parser.
    parse_jest(stdout)
}

#[must_use]
fn parse_pytest(stdout: &[u8]) -> TestResults {
    let v: Value = serde_json::from_slice(stdout).unwrap_or(Value::Null);
    let summary = v.get("summary");
    let passed = summary
        .and_then(|s| s.get("passed"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let failed = summary
        .and_then(|s| s.get("failed"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let skipped = summary
        .and_then(|s| s.get("skipped"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let duration_secs = v.get("duration").and_then(Value::as_f64).unwrap_or(0.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let duration_ms = (duration_secs * 1000.0) as u64;

    let mut failures = Vec::new();
    if let Some(tests) = v.get("tests").and_then(Value::as_array) {
        for t in tests {
            if t.get("outcome").and_then(Value::as_str) != Some("failed") {
                continue;
            }
            let nodeid = t.get("nodeid").and_then(Value::as_str).unwrap_or("");
            let (file, test_name) = split_pytest_nodeid(nodeid);
            let line = t
                .get("lineno")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            let longrepr = t
                .get("call")
                .and_then(|c| c.get("longrepr"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let stack = pytest_stack(t);

            failures.push(TestFailure {
                test_name,
                file,
                line,
                message: first_line(&longrepr),
                stack,
            });
        }
    }

    TestResults {
        passed: cast_u32(passed),
        failed: cast_u32(failed),
        skipped: cast_u32(skipped),
        duration_ms,
        failures,
    }
}

#[must_use]
fn parse_cargo(stdout: &[u8]) -> TestResults {
    let text = std::str::from_utf8(stdout).unwrap_or_default();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut duration_ms = 0u64;
    let mut failures = Vec::new();

    for line in text.lines() {
        let Ok(v): std::result::Result<Value, _> = serde_json::from_str(line) else {
            continue;
        };
        match (v.get("type").and_then(Value::as_str), v.get("event").and_then(Value::as_str)) {
            (Some("test"), Some("ok")) => passed += 1,
            (Some("test"), Some("failed")) => {
                failed += 1;
                let name = v
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let stdout_msg = v
                    .get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let (file, line) = cargo_parse_panic_location(&stdout_msg);

                failures.push(TestFailure {
                    test_name: name,
                    file,
                    line,
                    message: first_line(&stdout_msg),
                    stack: Vec::new(),
                });
            }
            (Some("test"), Some("ignored")) => skipped += 1,
            (Some("suite"), Some(_)) => {
                let secs = v.get("exec_time").and_then(Value::as_f64).unwrap_or(0.0);
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let ms = (secs * 1000.0) as u64;
                duration_ms = ms;
            }
            _ => {}
        }
    }

    TestResults {
        passed,
        failed,
        skipped,
        duration_ms,
        failures,
    }
}

// ------------ helpers ------------

fn cast_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}

fn parse_stack(raw: &str) -> Vec<(PathBuf, u32)> {
    let mut out = Vec::new();
    for line in raw.lines() {
        // Heuristic: look for "(FILE:LINE)" or "at FILE:LINE" or "at X (FILE:LINE)".
        let trimmed = line.trim();
        // Extract content in parens first.
        if let Some(open) = trimmed.rfind('(') {
            if let Some(close) = trimmed.rfind(')') {
                if close > open {
                    let inner = &trimmed[open + 1..close];
                    if let Some((file, line_str)) = inner.rsplit_once(':') {
                        if let Ok(line_no) = line_str.parse::<u32>() {
                            out.push((PathBuf::from(file), line_no));
                            continue;
                        }
                    }
                }
            }
        }
        // Fall back to "at FILE:LINE" trailing pattern.
        if let Some(rest) = trimmed.strip_prefix("at ") {
            if let Some((file, line_str)) = rest.rsplit_once(':') {
                if let Ok(line_no) = line_str.parse::<u32>() {
                    out.push((PathBuf::from(file), line_no));
                }
            }
        }
    }
    out
}

fn split_pytest_nodeid(nodeid: &str) -> (PathBuf, String) {
    // "tests/test_handler.py::test_foo" → (tests/test_handler.py, test_foo)
    match nodeid.split_once("::") {
        Some((file, test)) => (PathBuf::from(file), test.to_string()),
        None => (PathBuf::new(), nodeid.to_string()),
    }
}

fn pytest_stack(test: &Value) -> Vec<(PathBuf, u32)> {
    let mut out = Vec::new();
    if let Some(frames) = test
        .get("call")
        .and_then(|c| c.get("traceback"))
        .and_then(Value::as_array)
    {
        for frame in frames {
            let path = frame.get("path").and_then(Value::as_str).map(PathBuf::from);
            let line = frame
                .get("lineno")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok());
            if let (Some(p), Some(l)) = (path, line) {
                out.push((p, l));
            }
        }
    }
    out
}

fn cargo_parse_panic_location(stdout: &str) -> (PathBuf, u32) {
    // "thread 'x' panicked at '...', src/foo.rs:42:5"
    for line in stdout.lines() {
        if line.contains("panicked at") {
            // Find the last ", " then take what comes after.
            if let Some(after_comma) = line.rfind(", ") {
                let loc = &line[after_comma + 2..];
                // Format: path:line:col or path:line
                let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
                if parts.len() >= 2 {
                    // Line is parts[1] if col present, else parts[0]; path is parts[last]
                    let (path, line_str) = if parts.len() == 3 {
                        (parts[2], parts[1])
                    } else {
                        (parts[1], parts[0])
                    };
                    if let Ok(n) = line_str.parse::<u32>() {
                        return (PathBuf::from(path.trim()), n);
                    }
                }
            }
        }
    }
    (PathBuf::new(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(format!("tests/fixtures/runner_outputs/{name}")).expect("fixture missing")
    }

    #[test]
    fn jest_passing_parses_counts() {
        let r = parse_jest(&fixture("jest_passing.json"));
        assert_eq!(r.passed, 3);
        assert_eq!(r.failed, 0);
        assert_eq!(r.skipped, 0);
        assert!(r.failures.is_empty());
    }

    #[test]
    fn jest_failing_extracts_failure_file_line() {
        let r = parse_jest(&fixture("jest_failing.json"));
        assert_eq!(r.passed, 2);
        assert_eq!(r.failed, 1);
        assert_eq!(r.skipped, 1);
        assert_eq!(r.failures.len(), 1);
        let f = &r.failures[0];
        assert_eq!(f.test_name, "failing path");
        assert_eq!(f.file, PathBuf::from("/tmp/handler.test.js"));
        assert_eq!(f.line, 42);
        assert!(f.message.contains("expected 200"));
    }

    #[test]
    fn jest_failure_stack_has_frames() {
        let r = parse_jest(&fixture("jest_failing.json"));
        let f = &r.failures[0];
        assert!(!f.stack.is_empty(), "stack should have frames");
        assert!(f.stack.iter().any(|(p, _)| p.to_string_lossy().contains("handler.js")));
    }
}
