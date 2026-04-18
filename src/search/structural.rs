//! Structural search — graph-backed queries (callers, callees, outline, …).
//!
//! Thin wrapper over [`crate::graph::ops`] that formats results into
//! [`super::SearchHit`] with inline signatures.

// TODO(phase-1.5): callers_of, callees_of, outline, tests_for, libraries.
