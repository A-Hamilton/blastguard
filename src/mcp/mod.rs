//! MCP tool handlers over rmcp 1.x stdio transport — SPEC §3.
//!
//! Each of the three tools is a module sibling:
//! [`search`], [`apply_change`], [`run_tests`]. The rmcp server wiring lives
//! in [`server`]; the `blastguard://status` resource lives in [`status`].

pub mod apply_change;
pub mod run_tests;
pub mod search;
pub mod server;
pub mod status;
