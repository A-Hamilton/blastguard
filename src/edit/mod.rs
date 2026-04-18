//! `apply_change` tool backend — SPEC §3.2.
//!
//! Orchestrates: (1) disk edit via [`apply`], (2) reparse via
//! [`crate::parse`], (3) symbol diff via [`diff`], (4) cascade detection
//! via [`crate::graph::impact`], (5) bundled context via [`context`].
//!
//! Plan 4 wires the entry point into an rmcp `#[tool]` handler. For now
//! the orchestrator in [`apply::orchestrate`] returns a plain [`Result`]
//! that the caller can map into `CallToolResult { is_error: true, .. }`
//! on failure.

pub mod apply;
pub mod context;
pub mod diff;
pub mod request;

pub use request::{ApplyChangeRequest, ApplyChangeResponse, ApplyStatus, BundledContext, Change};
