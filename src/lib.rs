//! `BlastGuard` — Rust MCP server that lifts AI coding agents on SWE-bench Pro
//! via AST graph retrieval, cascade warnings, and test-failure attribution.
//!
//! Module layout follows `CLAUDE.md` file-conventions:
//!
//! - [`mcp`] — tool handlers and `isError` mapping over rmcp stdio
//! - [`graph`] — node/edge types, graph ops, cascade-impact analysis
//! - [`parse`] — tree-sitter drivers per language + symbol extraction
//! - [`index`] — parallel indexer, BLAKE3 Merkle cache, file watcher
//! - [`search`] — query dispatcher, structural/graph search, grep fallback
//! - [`runner`] — test runner detection, execution, output parsing
//! - [`session`] — in-memory per-server state used for failure attribution
//! - [`config`] — project config loader (`.blastguard/config.toml`)
//! - [`error`] — library-surface error type
//! - [`semantic`] (feature `semantic`) — embeddings + sqlite-vec

#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

pub mod config;
pub mod error;
pub mod graph;
pub mod index;
pub mod mcp;
pub mod parse;
pub mod runner;
pub mod search;
pub mod session;

#[cfg(feature = "semantic")]
pub mod semantic;

pub use error::{BlastGuardError, Result};
