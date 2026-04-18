//! `BlastGuard` binary entry point. Initialises tracing to stderr, loads the
//! project config, boots the indexer, and hands off to the rmcp stdio server.
//!
//! `println!`/`eprintln!` are forbidden in this crate — the stdio transport
//! owns stdout for the MCP wire protocol. Logs go to stderr via `tracing`.

use std::path::PathBuf;

use anyhow::Context;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    init_tracing();

    let project_root = resolve_project_root()?;
    tracing::info!(project_root = %project_root.display(), "BlastGuard starting");

    // Spin up a Tokio runtime and hand off to the async MCP boot sequence.
    // The rest of main is synchronous (tracing init, arg parsing), so we use
    // block_on rather than annotating main with #[tokio::main], which would
    // silently swallow the synchronous work above into the async context.
    let rt = tokio::runtime::Runtime::new().context("creating tokio runtime")?;
    rt.block_on(blastguard::mcp::server::run(&project_root))
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("BLASTGUARD_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,blastguard=info"));

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .with_ansi(false);

    if cfg!(debug_assertions) {
        subscriber.pretty().init();
    } else {
        subscriber.json().init();
    }
}

fn resolve_project_root() -> anyhow::Result<PathBuf> {
    let arg = std::env::args().nth(1);
    match arg {
        Some(path) => {
            let p = PathBuf::from(path);
            std::fs::canonicalize(&p).with_context(|| format!("canonicalizing {}", p.display()))
        }
        None => std::env::current_dir().context("reading current working directory"),
    }
}
