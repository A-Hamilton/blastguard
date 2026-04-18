//! rmcp 1.x server wiring — SPEC §2 architecture + §3.4 isError mapping.
//!
//! Phase 1.8 lands the full `#[tool_router]` wiring with stdio transport.
//! For now `run` brings up the indexer + session state and exits cleanly
//! with a tracing note so downstream phases can iterate without a broken
//! binary.

use std::path::Path;

use anyhow::Context;

use crate::config::Config;
use crate::index::indexer;
use crate::session::SessionState;

/// Binary entry point called from `src/main.rs`. The full stdio boot lands
/// in Phase 1.8; this placeholder loads config + warm-start index so the
/// rest of the modules can be wired up incrementally.
///
/// # Errors
/// Bubbles up config / indexer / runtime errors at the binary boundary.
pub fn run(project_root: &Path) -> anyhow::Result<()> {
    let _config = Config::load(project_root).context("loading .blastguard/config.toml")?;
    let _graph = indexer::warm_start(project_root).context("warm-starting index")?;
    let _session = SessionState::new();
    tracing::warn!(
        "Phase 1.8 rmcp stdio wiring not yet landed; exiting cleanly. \
         Build and run `cargo test` to exercise implemented modules."
    );
    Ok(())
}
