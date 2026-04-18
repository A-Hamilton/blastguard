//! Search dispatcher and backends — SPEC §3.1.
//!
//! Module layout (all populated incrementally across Plan 2):
//! - [`hit`] — `SearchHit` struct + centrality ranking helper (Task 1)
//! - [`query`] — `QueryKind` enum + `classify` regex ladder (Task 2)
//! - [`dispatcher`] — `dispatch` entry point routing `QueryKind` → backend (Tasks 2+)
//! - [`structural`] — graph-backed implementations of each `QueryKind` (Tasks 3-11)
//! - [`text`] — regex grep fallback via the `ignore` crate (Task 12)

pub mod dispatcher;
pub mod hit;
pub mod query;
pub mod structural;
pub mod text;

pub use dispatcher::dispatch;
pub use hit::SearchHit;
pub use query::QueryKind;
