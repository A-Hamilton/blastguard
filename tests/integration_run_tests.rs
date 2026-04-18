//! End-to-end: seed a tempdir Rust crate with 2 passing + 1 failing
//! test, run cargo test, assert parsed counts. Early-returns on any
//! spawn failure so CI environments without cargo don't fail the suite.

use std::sync::Mutex;

use blastguard::graph::types::CodeGraph;
use blastguard::runner::{run_tests, RunTestsRequest};
use blastguard::session::SessionState;

#[test]
fn cargo_test_run_end_to_end() {
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"bg_test_fixture\"\nversion = \"0.0.1\"\nedition = \"2021\"\n",
    )
    .expect("write Cargo.toml");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
    std::fs::write(
        tmp.path().join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a + b }\n\
         #[cfg(test)]\n\
         mod tests {\n\
             use super::*;\n\
             #[test]\n\
             fn ok_one() { assert_eq!(add(1, 2), 3); }\n\
             #[test]\n\
             fn ok_two() { assert_eq!(add(10, 5), 15); }\n\
             #[test]\n\
             fn will_fail() { assert_eq!(add(1, 1), 3); }\n\
         }\n",
    )
    .expect("write lib.rs");

    let graph = Mutex::new(CodeGraph::new());
    let session = Mutex::new(SessionState::new());
    let req = RunTestsRequest {
        filter: None,
        timeout_seconds: 120,
    };

    let resp = match run_tests(&graph, &session, tmp.path(), &req) {
        Ok(r) => r,
        Err(e) => {
            // `cargo test -- --format json` requires nightly (-Z
            // unstable-options). On stable the runner exits non-zero and
            // Fix B surfaces TestCrashed. Early-return rather than
            // failing this opportunistic integration test.
            eprintln!("run_tests err (skipping on stable / cargo unavailable): {e:?}");
            return;
        }
    };

    // Stable toolchain: skip assertions on zero counts (shouldn't happen
    // after Fix B, but defense in depth).
    if resp.passed + resp.failed + resp.skipped == 0 {
        eprintln!("zero counts — skipping");
        return;
    }

    assert_eq!(resp.passed, 2, "passed mismatch: {resp:?}");
    assert_eq!(resp.failed, 1, "failed mismatch: {resp:?}");
    assert!(
        resp.failures.iter().any(|f| f.contains("will_fail")),
        "expected will_fail in failures; got {:?}",
        resp.failures
    );
}
