//! rmcp 1.5 server wiring — SPEC §2 architecture + §3.4 `isError` mapping.
//!
//! [`BlastGuardServer`] holds shared state (graph, session, config) behind
//! `Arc<Mutex<_>>` so it can be cheaply cloned across tool-handler futures.
//! The [`ServerHandler`] impl is intentionally skeletal here — tool routers
//! and resource handlers land in Tasks 3-6. An empty impl compiles because
//! every `ServerHandler` method has a provided default.
//!
//! [`run`] is the async boot entry called from `main.rs` via
//! `tokio::runtime::Runtime::block_on`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::service::ServiceExt as _;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::graph::CodeGraph;
use crate::index::indexer;
use crate::session::SessionState;

// ── BlastGuardServer ────────────────────────────────────────────────────────

/// Shared server state. Cheap to clone — all heavy data lives behind `Arc`.
///
/// Constructed once in [`run`] and passed directly to `serve()`. Tool handlers
/// (added in subsequent tasks) will receive a clone of this struct per-call.
// Fields are read by tool handlers that land in Tasks 3-6; suppress the
// dead_code lint until those modules exist.
#[expect(dead_code, reason = "fields consumed by tool handlers in Tasks 3-6")]
#[derive(Clone)]
pub struct BlastGuardServer {
    /// In-memory code graph, rebuilt from cache on warm start.
    pub(crate) graph: Arc<Mutex<CodeGraph>>,
    /// Per-session mutable state: edited files, last test run, etc.
    pub(crate) session: Arc<Mutex<SessionState>>,
    /// Absolute path to the project root passed on the command line.
    pub(crate) project_root: PathBuf,
    /// Project configuration loaded from `.blastguard/config.toml`.
    pub(crate) config: Arc<Config>,
}

impl BlastGuardServer {
    /// Construct a new server, wrapping `graph` and a fresh [`SessionState`]
    /// in `Arc<Mutex<_>>` for shared ownership across async handlers.
    ///
    /// `config` is already cheaply clonable via `Arc`; the raw [`Config`] is
    /// wrapped here so callers don't need to construct the `Arc` themselves.
    #[must_use]
    pub fn new(graph: CodeGraph, project_root: PathBuf, config: Config) -> Self {
        Self {
            graph: Arc::new(Mutex::new(graph)),
            session: Arc::new(Mutex::new(SessionState::new())),
            project_root,
            config: Arc::new(config),
        }
    }
}

// ── ServerHandler impl ───────────────────────────────────────────────────────

impl ServerHandler for BlastGuardServer {
    /// Advertise the server identity and capabilities to connecting clients.
    ///
    /// Tool and resource capabilities are added when the `#[tool_router]`
    /// macro and resource handlers land in Tasks 3-6.
    fn get_info(&self) -> ServerInfo {
        // `ServerInfo` = `InitializeResult`, which is `#[non_exhaustive]`.
        // Use the provided builder methods: `new(capabilities)` sets a default
        // protocol version; chain `.with_server_info()` and `.with_instructions()`.
        ServerInfo::new(ServerCapabilities::builder().build())
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "BlastGuard: AST graph search, cascade-aware edits, and test-failure \
                 attribution for AI coding agents. Tools: search, apply_change, run_tests.",
            )
    }
}

// ── Boot entry point ─────────────────────────────────────────────────────────

/// Async boot entry: load config, warm-start the index, stand up the rmcp
/// stdio server, and block until the MCP client disconnects.
///
/// This is the only public async surface in this module. `main.rs` wraps it
/// with `tokio::runtime::Runtime::block_on` because the rest of the binary
/// is synchronous (tracing init, arg parsing).
///
/// # Errors
///
/// Propagates:
/// - [`crate::error::BlastGuardError`] from config load or indexer warm-start.
/// - [`rmcp::ServerInitializeError`] if the MCP handshake fails.
/// - [`tokio::task::JoinError`] if the background service task panics.
pub async fn run(project_root: &Path) -> anyhow::Result<()> {
    let config = Config::load(project_root).context("loading .blastguard/config.toml")?;

    let graph = indexer::warm_start(project_root).context("warm-starting index")?;

    let server = BlastGuardServer::new(graph, project_root.to_path_buf(), config);

    tracing::info!(
        project_root = %project_root.display(),
        version = env!("CARGO_PKG_VERSION"),
        "BlastGuard MCP server starting on stdio"
    );

    // `rmcp::transport::io::stdio()` returns `(tokio::io::Stdin, Stdout)`.
    // Passing the tuple satisfies `IntoTransport<RoleServer, _, _>` via the
    // blanket `(AsyncRead, AsyncWrite)` impl in rmcp's transport layer.
    let transport = rmcp::transport::io::stdio();

    let service = server
        .serve(transport)
        .await
        .context("rmcp server initialization failed")?;

    tracing::info!("BlastGuard MCP server ready — awaiting client requests");

    service.waiting().await.context("MCP service terminated with error")?;

    tracing::info!("BlastGuard MCP server shutting down cleanly");
    Ok(())
}
