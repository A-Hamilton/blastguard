//! Search dispatcher and backends — SPEC §3.1.
//!
//! The dispatcher parses the `query` string, classifies it by pattern, and
//! routes to the structural (graph) backend or the regex-grep fallback.
//! Phase 2 adds the semantic backend behind the `semantic` feature.

pub mod dispatcher;
pub mod structural;
pub mod text;

pub use dispatcher::{dispatch, SearchHit};
