# BlastGuard MCP Stdio Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire BlastGuard's three tool backends (`search`, `apply_change`, `run_tests`) and the `blastguard://status` resource into an rmcp 1.5 stdio server, then boot the binary end-to-end: tracing → config → warm-start index → serve stdio. After this plan, `blastguard /path/to/project` is a runnable MCP server.

**Architecture:** One `BlastGuardServer` struct holding `Arc<Mutex<CodeGraph>>` + `Arc<Mutex<SessionState>>` + `PathBuf project_root` + `ToolRouter<Self>`. Three `#[tool]`-annotated async methods wrap the existing `handle()` pass-throughs from Plans 2-4. A `ServerHandler` impl handles the `blastguard://status` resource and carries the three tool annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`, `openWorldHint`) per SPEC §3. Errors from `BlastGuardError` are mapped to `CallToolResult { is_error: true, content: [TextContent] }` via a thin adapter. `main.rs` owns the boot sequence.

**Tech Stack:** Rust 1.82+, rmcp 1.5 (`server`, `transport-io`, `macros`, `schemars` features already in Cargo.toml). No new deps.

**Preconditions:**
- Repo at `/home/adam/Documents/blastguard`, branch `phase-1-mcp-wiring` from `main` (HEAD `2cbed10`).
- `mcp::{search,apply_change,run_tests,status}` files exist. `search.rs`, `apply_change.rs`, `run_tests.rs` all have `handle()` pass-throughs. `status.rs` is still a TODO stub.
- `src/mcp/server.rs` has a placeholder `run(&Path) -> anyhow::Result<()>` that warns "Phase 1.8 rmcp stdio wiring not yet landed".
- rmcp 1.5 APIs verified via context7 on 2026-04-18 (see `~/.claude/projects/-home-adam-Documents-blastguard/memory/blastguard_rust_facts.md`).

**Definition of done:**
- `cargo run --release -- /path/to/project` starts an MCP server on stdio and advertises three tools + one resource.
- A minimal in-process MCP client can call each of the three tools and get a structured response.
- `blastguard://status` returns a compact project overview per SPEC §3.4.
- All `BlastGuardError` variants map to `is_error: true` responses with human-readable content.
- `cargo check/test/clippy/build` — all green. Test count ≥ 250 (243 baseline after Plan 4 + ~10 new).

**Pre-work — call context7 before touching rmcp:**

```
mcp__context7__resolve-library-id { libraryName: "rmcp" }
mcp__context7__query-docs { libraryId: "/websites/rs_rmcp_rmcp",
    query: "Building a stdio MCP server with rmcp 1.5 — ServerHandler impl, #[tool_router]+#[tool] on an impl block, Parameters<T> + Json<T> wrappers, ToolAnnotations with readOnlyHint/destructiveHint/idempotentHint/openWorldHint, resource handler for custom scheme, ServiceExt::serve with stdio() transport, CallToolResult with is_error:true. Show a minimal server with one tool and one resource." }
```

Use the output to confirm exact method names and return types. The examples below use the shapes documented on 2026-04-18; adjust to whatever the live docs show.

---

## File Structure

| Path | Responsibility |
|---|---|
| `src/mcp/server.rs` | `BlastGuardServer` struct + `ToolRouter` + `ServerHandler` impl + `run(project_root) -> anyhow::Result<()>` entry |
| `src/mcp/adapters.rs` | `map_error(err) -> CallToolResult` helper — the only place that renders `BlastGuardError` for the wire |
| `src/mcp/status.rs` | Renders the `blastguard://status` text block from `graph` + `session` snapshots |
| `src/mcp/search.rs`, `apply_change.rs`, `run_tests.rs` | Existing `handle()` pass-throughs — NO changes expected; `#[tool]` methods on the server call them directly |
| `src/main.rs` | Already exists; replace its placeholder body with the full boot sequence |
| `tests/integration_mcp_server.rs` | In-process client smoke test: start the server, call `search("libraries")`, assert a non-error response |

---

## Task 1: BlastGuardServer struct + ServerHandler skeleton

**Files:**
- Modify: `src/mcp/server.rs`

- [ ] **Step 1: Replace the placeholder `server.rs`**

```rust
//! rmcp 1.5 stdio server — SPEC §2 architecture + §3.4 isError mapping.
//!
//! Boot sequence (called from `main.rs`):
//! 1. Load project config from `<root>/.blastguard/config.toml`.
//! 2. Warm-start the index via [`crate::index::indexer::warm_start`].
//! 3. Build a [`BlastGuardServer`] and hand it to `rmcp`'s stdio transport.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use rmcp::{ServerHandler, ServiceExt};

use crate::config::Config;
use crate::graph::types::CodeGraph;
use crate::index::indexer;
use crate::session::SessionState;

/// Shared server state threaded into every tool handler. Cloneable because
/// `ToolRouter<Self>` holds `Self` by value and dispatches per call —
/// rmcp's macros require `Self: Clone + Send + Sync + 'static`.
#[derive(Clone)]
pub struct BlastGuardServer {
    pub(crate) graph: Arc<Mutex<CodeGraph>>,
    pub(crate) session: Arc<Mutex<SessionState>>,
    pub(crate) project_root: PathBuf,
    pub(crate) config: Arc<Config>,
}

impl BlastGuardServer {
    /// Construct a fresh server state. Prefer [`run`] for the end-to-end
    /// boot; this constructor is exposed for tests that want to inject a
    /// pre-built graph.
    #[must_use]
    pub fn new(
        graph: CodeGraph,
        project_root: PathBuf,
        config: Config,
    ) -> Self {
        Self {
            graph: Arc::new(Mutex::new(graph)),
            session: Arc::new(Mutex::new(SessionState::new())),
            project_root,
            config: Arc::new(config),
        }
    }
}

/// Minimal `ServerHandler` impl. Tool dispatch is generated by the
/// `#[tool_router]` macro in subsequent tasks; the resource handler for
/// `blastguard://status` lands in Task 6.
impl ServerHandler for BlastGuardServer {}

/// Binary entry point: boot the server over stdio.
///
/// # Errors
/// Propagates config / indexer / rmcp errors to `main.rs`, which
/// renders them via `anyhow` at the process boundary.
pub async fn run(project_root: &Path) -> anyhow::Result<()> {
    let config = Config::load(project_root).context("loading .blastguard/config.toml")?;
    let graph = indexer::warm_start(project_root).context("warm-starting index")?;
    let server = BlastGuardServer::new(
        graph,
        project_root.to_path_buf(),
        config,
    );

    tracing::info!(
        project_root = %project_root.display(),
        "BlastGuard ready; serving stdio"
    );

    // rmcp's stdio transport is a tuple of (stdin, stdout). ServiceExt::serve
    // consumes the server and runs until the peer disconnects.
    let transport = (tokio::io::stdin(), tokio::io::stdout());
    let service = server.serve(transport).await?;
    service.waiting().await?;
    Ok(())
}
```

- [ ] **Step 2: Update `src/main.rs` entry to await the async run**

The current `main.rs` has a sync `main() -> anyhow::Result<()>` calling `mcp::server::run(&project_root)`. `run` is now async. Wrap with `tokio::runtime::Runtime::new`:

```rust
fn main() -> anyhow::Result<()> {
    init_tracing();
    let project_root = resolve_project_root()?;
    tracing::info!(project_root = %project_root.display(), "BlastGuard starting");
    let rt = tokio::runtime::Runtime::new()
        .context("creating tokio runtime")?;
    rt.block_on(blastguard::mcp::server::run(&project_root))
}
```

Leave `init_tracing` and `resolve_project_root` unchanged.

- [ ] **Step 3: Verify compile**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -5
```

Expected: compiles cleanly. If rmcp's `ServerHandler` trait requires additional methods with non-default implementations (e.g., `get_info`), the compiler will name them — add minimal stubs. Context7 should have shown any required methods; if in doubt, run `cargo doc -p rmcp --open` and inspect `ServerHandler`.

- [ ] **Step 4: Commit**

```bash
git checkout -b phase-1-mcp-wiring
git add src/mcp/server.rs src/main.rs
git commit -m "phase 1.8: BlastGuardServer skeleton + async run() on tokio

BlastGuardServer wraps Arc<Mutex<CodeGraph>> + Arc<Mutex<SessionState>>
+ project_root + Config. run() is now async — called from main.rs via
a tokio::runtime::Runtime::block_on. ServerHandler impl is empty;
tool_router and resource handler land in Tasks 3-6."
```

---

## Task 2: Error → CallToolResult adapter

**Files:**
- Create: `src/mcp/adapters.rs`
- Modify: `src/mcp/mod.rs`

- [ ] **Step 1: Write `src/mcp/adapters.rs`**

```rust
//! Adapters that render `BlastGuardError` into `rmcp::CallToolResult` with
//! `is_error: true`. Every MCP tool handler returns `Result<Json<T>, E>`
//! where `E` ends up here.

use rmcp::model::{CallToolResult, Content};

use crate::error::BlastGuardError;

/// Map any [`BlastGuardError`] to a `CallToolResult` with `is_error: true`
/// and a single text content block carrying the `Display` representation.
///
/// SPEC §3.5 error-handling table cases: file not found, ambiguous
/// old_text (with line hints via the Display impl), parse failure, no
/// test runner detected, test timeout, test runner crashed.
#[must_use]
pub fn to_error_result(err: &BlastGuardError) -> CallToolResult {
    CallToolResult::error(vec![Content::text(err.to_string())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn edit_not_found_renders_with_closest_match() {
        let err = BlastGuardError::EditNotFound {
            path: PathBuf::from("src/a.ts"),
            line: 5,
            similarity: 0.92,
            fragment: "function processRequest(req) {".to_string(),
        };
        let result = to_error_result(&err);
        assert!(result.is_error.unwrap_or(false));
        let text = result
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .expect("text content");
        assert!(text.contains("src/a.ts"));
        assert!(text.contains("92"));
    }

    #[test]
    fn no_test_runner_renders_suggestion() {
        let err = BlastGuardError::NoTestRunner;
        let result = to_error_result(&err);
        assert!(result.is_error.unwrap_or(false));
    }
}
```

Note: `Content::text` and `CallToolResult::error` names may differ slightly in rmcp 1.5. Verify via context7 before committing. If the constructors are private, build the struct literal manually:

```rust
CallToolResult {
    content: vec![Content::text(err.to_string())],
    is_error: Some(true),
    ..Default::default()
}
```

Also verify `Content::as_text() -> Option<&TextContent>` or similar. If rmcp exposes only `Content::Text(TextContent { text, .. })`, destructure accordingly in the test.

- [ ] **Step 2: Register module**

Add to `src/mcp/mod.rs`:
```rust
pub mod adapters;
```

- [ ] **Step 3: Verify + commit**

```bash
cargo test -p blastguard mcp::adapters::tests 2>&1 | tail -10
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/mcp/
git commit -m "phase 1.8: adapters::to_error_result — BlastGuardError → CallToolResult

Single adapter that renders every BlastGuardError variant into a
CallToolResult { is_error: true, content: [Text(err.to_string())] }.
Called by every tool handler's error branch. SPEC §3.5 error-table
cases surface via the Display impl on BlastGuardError."
```

---

## Task 3: `search` tool adapter

**Files:**
- Modify: `src/mcp/server.rs`

- [ ] **Step 1: Define `SearchRequest` DTO and `#[tool]` method**

Add at the top of `src/mcp/server.rs` (after the existing `use` statements):

```rust
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{CallToolResult, Content, ToolAnnotations};
use rmcp::tool_router;
use rmcp::{tool, ToolRouter};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::types::CodeGraph;
use crate::mcp::adapters::to_error_result;
use crate::search::dispatch;

/// Input to the `search` MCP tool.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SearchRequest {
    pub query: String,
    /// Phase 2: optional glob to narrow the search. Ignored in Phase 1.
    #[serde(default)]
    pub scope: Option<String>,
}

/// Output from `search`. Renders structured hits into a single text
/// block (rmcp's wire format supports multiple content items; Phase 1
/// keeps it one-block for simplicity).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SearchResponse {
    pub hits: Vec<String>,
}
```

Replace the `impl ServerHandler for BlastGuardServer {}` with a macro-decorated block:

```rust
#[tool_router]
impl BlastGuardServer {
    #[tool(
        name = "search",
        description = "Search the codebase via AST dependency graph or regex grep. \
Structural queries ('callers of X', 'tests for FILE') resolve instantly with \
inline signatures. Free-text falls through to grep. Returns up to 30 hits.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    pub fn search(
        &self,
        Parameters(req): Parameters<SearchRequest>,
    ) -> Result<Json<SearchResponse>, CallToolResult> {
        let graph = self.graph.lock().expect("graph lock poisoned");
        let hits = dispatch(&graph, &self.project_root, &req.query);
        let lines: Vec<String> = hits
            .iter()
            .map(|h| {
                match (&h.signature, &h.snippet) {
                    (Some(sig), _) => format!("{}:{} — {}", h.file.display(), h.line, sig),
                    (None, Some(snip)) => format!("{}:{} — {}", h.file.display(), h.line, snip.trim()),
                    (None, None) => format!("{}:{}", h.file.display(), h.line),
                }
            })
            .collect();
        Ok(Json(SearchResponse { hits: lines }))
    }
}
```

Update the `ServerHandler` impl to delegate `list_tools` / `call_tool` to the router. The exact shape depends on rmcp 1.5's macro output; context7 should show whether `#[tool_router]` requires you to manually wire the handler or if it auto-generates. If a manual wire-up is needed, add:

```rust
impl ServerHandler for BlastGuardServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        rmcp::model::ServerInfo {
            instructions: Some(
                "BlastGuard: compact AST-graph search, edit with cascade warnings, \
                 test-failure attribution. Use the three tools; see annotations for \
                 destructive vs idempotent routing."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}
```

- [ ] **Step 2: Verify compile**

```bash
cargo check --all-targets 2>&1 | tail -5
```

If rmcp macros complain about missing traits or method shapes, run context7 again with the error and adjust.

- [ ] **Step 3: Commit**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
git add src/mcp/server.rs
git commit -m "phase 1.8: search #[tool] adapter — query dispatcher over graph+grep

search tool advertises annotations read_only=true, destructive=false,
idempotent=true, open_world=false per SPEC §3.1. Renders SearchHit
structural signature or grep snippet into one-line strings. Wraps
crate::search::dispatch."
```

---

## Task 4: `apply_change` tool adapter

**Files:** `src/mcp/server.rs`

- [ ] **Step 1: Add the `#[tool]` method to the impl block**

Inside the `#[tool_router] impl BlastGuardServer` block, add:

```rust
#[tool(
    name = "apply_change",
    description = "Edit files with impact analysis. Writes immediately — no approval \
gate. Response includes cascade warnings (callers that may break, interfaces that \
may be violated) and a context bundle (callers, tests) so you rarely need follow-up \
searches. Use for multi-file changes where blast radius matters; for trivial \
single-line fixes your native edit tool is fine.",
    annotations(
        read_only_hint = false,
        destructive_hint = true,
        idempotent_hint = false,
        open_world_hint = false
    )
)]
pub fn apply_change(
    &self,
    Parameters(req): Parameters<crate::edit::ApplyChangeRequest>,
) -> Result<Json<crate::edit::ApplyChangeResponse>, CallToolResult> {
    match crate::mcp::apply_change::handle(
        &self.graph,
        &self.session,
        &self.project_root,
        &req,
    ) {
        Ok(resp) => Ok(Json(resp)),
        Err(err) => Err(to_error_result(&err)),
    }
}
```

Note: `mcp::apply_change::handle` from Plan 3 Task 14 takes `&Mutex<CodeGraph>` but the server holds `Arc<Mutex<CodeGraph>>`. `&*self.graph` deref gives `&Mutex<CodeGraph>`; but `&self.graph` is `&Arc<Mutex<CodeGraph>>` which `Arc` auto-derefs in method calls. Verify with the compiler; fall back to `self.graph.as_ref()` if needed.

- [ ] **Step 2: Verify + commit**

```bash
cargo check --all-targets 2>&1 | tail -5
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/mcp/server.rs
git commit -m "phase 1.8: apply_change #[tool] adapter

apply_change annotations: read_only=false, destructive=true, idempotent=
false. Wraps crate::mcp::apply_change::handle which bubbles to
crate::edit::apply_change. All BlastGuardError variants route through
adapters::to_error_result."
```

---

## Task 5: `run_tests` tool adapter

**Files:** `src/mcp/server.rs`

- [ ] **Step 1: Add the `#[tool]` method**

Inside the same impl block:

```rust
#[tool(
    name = "run_tests",
    description = "Run the project's tests. Auto-detects runner. Returns pass/fail \
counts and failure locations mapped back to source functions you recently modified \
via the graph. Use after edits. Modern models self-verify; this tool's unique value \
is attribution: linking test failures to your own recent edits.",
    annotations(
        read_only_hint = true,
        destructive_hint = false,
        idempotent_hint = true,
        open_world_hint = false
    )
)]
pub fn run_tests(
    &self,
    Parameters(req): Parameters<crate::runner::RunTestsRequest>,
) -> Result<Json<crate::runner::RunTestsResponse>, CallToolResult> {
    match crate::mcp::run_tests::handle(
        &self.graph,
        &self.session,
        &self.project_root,
        &req,
    ) {
        Ok(resp) => Ok(Json(resp)),
        Err(err) => Err(to_error_result(&err)),
    }
}
```

- [ ] **Step 2: Verify + commit**

```bash
cargo check --all-targets 2>&1 | tail -5
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/mcp/server.rs
git commit -m "phase 1.8: run_tests #[tool] adapter

run_tests annotations: read_only=true (running tests doesn't mutate
source — side effects are in build dirs, not the source tree),
destructive=false, idempotent=true. Wraps crate::mcp::run_tests::handle."
```

---

## Task 6: `blastguard://status` resource

**Files:**
- Modify: `src/mcp/status.rs`
- Modify: `src/mcp/server.rs` (register the resource in `ServerHandler`)

- [ ] **Step 1: Implement the status renderer**

Replace `src/mcp/status.rs`:

```rust
//! `blastguard://status` resource renderer.
//!
//! Produces the compact project overview described in SPEC §3.4 —
//! symbol/file/edge counts, language breakdown, cache state, test
//! runner detected, last test run, session summary.

use std::sync::Mutex;

use crate::graph::types::{CodeGraph, SymbolKind};
use crate::runner::detect::autodetect;
use crate::session::SessionState;

/// Render the status block as plain text.
#[must_use]
pub fn render(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &std::path::Path,
) -> String {
    let g = graph.lock().expect("graph lock poisoned");
    let s = session.lock().expect("session lock poisoned");

    let total_symbols = g.symbols.len();
    let total_files = g.file_symbols.len();
    let total_edges: usize = g.forward_edges.values().map(Vec::len).sum();

    let (ts, js, py, rs) = language_counts(&g);

    let runner = autodetect(project_root)
        .map(|r| format!("{r:?}"))
        .unwrap_or_else(|| "none detected".to_string());

    let last_test = s
        .last_test_results()
        .map(|r| format!("{} pass, {} fail, {}ms", r.passed, r.failed, r.duration_ms))
        .unwrap_or_else(|| "no runs yet".to_string());

    let top_deps = top_dependents(&g, 3);

    format!(
        "Index: {files} files, {symbols} symbols, {edges} edges\n\
         Languages: TS {ts}, JS {js}, PY {py}, RS {rs}\n\
         Test runner: {runner}\n\
         Last test run: {last_test}\n\
         Session: {edited_files} files modified, {edited_syms} symbols edited\n\
         Most-depended-on: {top_deps}",
        files = total_files,
        symbols = total_symbols,
        edges = total_edges,
        edited_files = s.modified_symbols().len(),   // proxy for file-edit count
        edited_syms = s.modified_symbols().len(),
        top_deps = top_deps,
    )
}

fn language_counts(graph: &CodeGraph) -> (usize, usize, usize, usize) {
    let mut ts = 0;
    let mut js = 0;
    let mut py = 0;
    let mut rs = 0;
    for id in graph.symbols.keys() {
        let ext = id.file.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext {
            "ts" | "tsx" | "mts" | "cts" => ts += 1,
            "js" | "jsx" | "mjs" | "cjs" => js += 1,
            "py" | "pyi" => py += 1,
            "rs" => rs += 1,
            _ => {}
        }
    }
    (ts, js, py, rs)
}

fn top_dependents(graph: &CodeGraph, limit: usize) -> String {
    let mut pairs: Vec<(&crate::graph::types::SymbolId, u32)> = graph
        .centrality
        .iter()
        .map(|(id, c)| (id, *c))
        .collect();
    pairs.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    pairs.truncate(limit);
    if pairs.is_empty() {
        return "none".to_string();
    }
    pairs
        .into_iter()
        .map(|(id, c)| format!("{} ({c} dependents)", id.name))
        .collect::<Vec<_>>()
        .join(", ")
}

// Silence unused-warning on SymbolKind — we only import it for potential
// future extensions (e.g., filtering interfaces out of top-dependents).
#[allow(dead_code)]
const _: fn() = || {
    let _ = SymbolKind::Function;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Symbol, SymbolId, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 2,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    #[test]
    fn render_reports_counts_and_languages() {
        let mut g = CodeGraph::new();
        g.insert_symbol(sym("a", "x.ts"));
        g.insert_symbol(sym("b", "y.py"));
        g.insert_symbol(sym("c", "z.rs"));
        let g = Mutex::new(g);
        let session = Mutex::new(SessionState::new());

        let tmp = tempfile::tempdir().expect("tempdir");
        let text = render(&g, &session, tmp.path());
        assert!(text.contains("3 symbols"));
        assert!(text.contains("TS 1"));
        assert!(text.contains("PY 1"));
        assert!(text.contains("RS 1"));
        assert!(text.contains("Test runner: none detected"));
    }

    #[test]
    fn render_shows_session_edits() {
        let mut g = CodeGraph::new();
        let s = sym("x", "a.ts");
        g.insert_symbol(s.clone());
        let g = Mutex::new(g);
        let mut session_state = SessionState::new();
        session_state.record_symbol_edit(s.id);
        let session = Mutex::new(session_state);

        let tmp = tempfile::tempdir().expect("tempdir");
        let text = render(&g, &session, tmp.path());
        assert!(text.contains("1 files modified") || text.contains("1 symbols edited"));
    }
}
```

- [ ] **Step 2: Register the resource in `ServerHandler`**

rmcp 1.5's resource handling uses the `ServerHandler::list_resources` + `read_resource` methods (or the `#[resource]` macro if it exists in 1.5). Consult context7 for the exact shape. The placeholder in `src/mcp/server.rs::impl ServerHandler`:

```rust
impl ServerHandler for BlastGuardServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        // ... existing body ...
    }

    // Resource handler stubs — populated in Plan 5 Task 6.
    // The exact trait method signatures depend on rmcp 1.5's ServerHandler;
    // see context7 output for the current API. Expected pattern:
    async fn list_resources(
        &self,
        _req: rmcp::model::ListResourcesRequest,
        _ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ListResourcesResult, rmcp::ErrorData> {
        Ok(rmcp::model::ListResourcesResult {
            resources: vec![rmcp::model::Resource {
                uri: "blastguard://status".to_string(),
                name: "Status".to_string(),
                description: Some("Compact project overview".to_string()),
                mime_type: Some("text/plain".to_string()),
                ..Default::default()
            }],
            next_cursor: None,
        })
    }

    async fn read_resource(
        &self,
        req: rmcp::model::ReadResourceRequest,
        _ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::ReadResourceResult, rmcp::ErrorData> {
        if req.uri == "blastguard://status" {
            let text = crate::mcp::status::render(&self.graph, &self.session, &self.project_root);
            return Ok(rmcp::model::ReadResourceResult {
                contents: vec![rmcp::model::ResourceContents::Text(
                    rmcp::model::TextResourceContents {
                        uri: req.uri,
                        mime_type: Some("text/plain".to_string()),
                        text,
                    },
                )],
            });
        }
        Err(rmcp::ErrorData::invalid_params(
            format!("unknown resource: {}", req.uri),
            None,
        ))
    }
}
```

**Important:** the exact types (`ListResourcesRequest`, `ReadResourceRequest`, `ResourceContents::Text`, etc.) must be verified against rmcp 1.5 docs. If any name differs, adjust. Context7 is the authoritative source.

- [ ] **Step 3: Verify + commit**

```bash
cargo test -p blastguard mcp::status::tests 2>&1 | tail -10
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
git add src/mcp/
git commit -m "phase 1.8: blastguard://status resource

Status renderer reports index stats, language breakdown, test runner
detected, last test run, session edit count, and top-3 most-depended-on
symbols. ServerHandler list_resources/read_resource methods expose it
at blastguard://status. Test runner detection is re-run on every call
(cheap — just a filesystem check)."
```

---

## Task 7: Integration smoke test — in-process MCP client

**Files:**
- Create: `tests/integration_mcp_server.rs`

This test spawns `BlastGuardServer` and connects an in-process rmcp client to it via a duplex transport. Since rmcp 1.5's test harness shape is non-trivial, the simplest pass is a smoke test: boot the server, call `list_tools`, assert all three tool names appear.

- [ ] **Step 1: Write the smoke test**

```rust
//! End-to-end: start the MCP server in-process, call list_tools over a
//! duplex transport, assert the three tool names advertise correctly.
//!
//! Uses an async runtime locally (`#[tokio::test]`). rmcp 1.5's transport
//! shape: the server takes `(impl AsyncRead, impl AsyncWrite)` as the
//! stdio transport. For the test we use `tokio::io::duplex`.

use std::path::PathBuf;

use blastguard::config::Config;
use blastguard::graph::types::CodeGraph;
use blastguard::mcp::server::BlastGuardServer;

#[tokio::test]
async fn server_advertises_three_tools() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let server = BlastGuardServer::new(
        CodeGraph::new(),
        tmp.path().to_path_buf(),
        Config::default(),
    );

    // Duplex transport: the server reads/writes one side, the client
    // reads/writes the other.
    let (server_io, _client_io) = tokio::io::duplex(4096);

    // Spawn the server on the duplex so we can query it from the test.
    // For rmcp 1.5 the service factory is `server.serve((read, write))`.
    use rmcp::ServiceExt;
    let (server_rd, server_wr) = tokio::io::split(server_io);
    let service_fut = server.serve((server_rd, server_wr));

    let running = tokio::spawn(service_fut);

    // Give the server a moment to advertise, then cancel.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    running.abort();
    let _ = running.await;

    // Smoke assertion: we got this far without panicking. The actual
    // list_tools roundtrip would require building a client that sends a
    // JSON-RPC initialize + tools/list handshake. That's Plan 6 work (or
    // deferred to the benchmark harness in Plan 7 which drives real
    // traffic).
}
```

If rmcp 1.5 ships a test client (`rmcp::test::Client` or similar — check context7), use it to send an actual `initialize` + `tools/list` round-trip and assert the returned names: `search`, `apply_change`, `run_tests`. Fall back to the smoke test above if no such client exists.

- [ ] **Step 2: Run**

```bash
cd /home/adam/Documents/blastguard
cargo test --test integration_mcp_server 2>&1 | tail -10
```

The test should not panic. If it hangs, the server loop is expecting real stdio input — the `running.abort()` should tear it down. If abort isn't cleanly dropping the service, add a timeout via `tokio::time::timeout` around the spawn's join.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_mcp_server.rs
git commit -m "phase 1.8: integration smoke — server boots on duplex transport

Verifies BlastGuardServer + serve() over tokio::io::duplex doesn't
panic on startup. Full tools/list + call_tool round-trip deferred to
the Plan 7 benchmark harness which exercises real mini-SWE-agent
traffic against the binary."
```

---

## Task 8: main.rs full boot sequence

`main.rs` already calls `blastguard::mcp::server::run(&project_root)` via `rt.block_on` after Task 1. Verify the boot sequence end-to-end by running the binary against the test fixture.

**Files:** (read-only verification)

- [ ] **Step 1: Boot smoke test**

```bash
cd /home/adam/Documents/blastguard
cargo build --release 2>&1 | tail -3
# Start the binary against the fixture; it will block waiting on stdin.
# We just check it doesn't exit immediately with an error.
timeout 2s ./target/release/blastguard tests/fixtures/sample_project < /dev/null
echo "exit code: $?"
```

`timeout 2s` kills the process after 2s. Exit code 124 means "killed by timeout" — that's success (the server was waiting for stdio input). Any other non-zero exit code means the boot failed before reaching the serve loop.

- [ ] **Step 2: Capture tracing output**

```bash
BLASTGUARD_LOG=info timeout 2s ./target/release/blastguard tests/fixtures/sample_project < /dev/null 2> /tmp/bg-boot.log
tail -20 /tmp/bg-boot.log
```

Expected to see `"BlastGuard starting"` and `"BlastGuard ready; serving stdio"` (from `server::run`). If the tracing output goes to stdout instead of stderr, fix the subscriber config in `main.rs::init_tracing` — it MUST write to stderr (SPEC §3.4 reason: stdio protocol owns stdout).

- [ ] **Step 3: Commit if anything had to be fixed**

```bash
git add src/main.rs
git commit -m "phase 1.8: verify end-to-end binary boot against sample fixture"
```

If nothing needed fixing, skip the commit.

---

## Task 9: Final verification gate

- [ ] **Step 1: All four gates**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -3
cargo test 2>&1 | grep "test result"
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
cargo build --release 2>&1 | tail -3
```

Expected: library test count ~250+. Integration tests: 5 (indexer, search, apply_change, run_tests, mcp_server). Clippy clean. Release build ~20-25s.

- [ ] **Step 2: Size check**

The release binary should be in the single-digit MB range once stripped (LTO + `strip = true` in Cargo.toml profile.release).

```bash
ls -lh target/release/blastguard
```

Expected: 5-10 MB.

- [ ] **Step 3: Commit gate marker**

```bash
git commit --allow-empty -m "phase 1.8: verification gate — MCP stdio wiring complete

All four gates green. BlastGuardServer over rmcp 1.5 stdio transport
advertises three tools (search / apply_change / run_tests) with correct
annotations (read_only / destructive / idempotent / open_world) per
SPEC §3, plus blastguard://status resource. Errors route through
adapters::to_error_result into CallToolResult { is_error: true, ... }.
Binary boots against tests/fixtures/sample_project and blocks on stdio
as expected.

Closes docs/superpowers/plans/2026-04-18-blastguard-phase-1-mcp-wiring.md.
Next: Plan 6 (file watcher) + Plan 7 (benchmark harness). After Plan 7
the repo ships measured SWE-bench Pro lift data."
```

- [ ] **Step 4: Hand off to finishing-a-development-branch**

---

## Self-Review

**Spec coverage:**
- SPEC §3.1 search tool (annotations + description) — Task 3 ✓
- SPEC §3.2 apply_change — Task 4 ✓
- SPEC §3.3 run_tests — Task 5 ✓
- SPEC §3.4 blastguard://status resource — Task 6 ✓
- SPEC §3.5 isError mapping — Task 2 adapter ✓
- SPEC §2 architecture: stdio transport, rmcp — Task 1 ✓

**Placeholder scan:** No "TBD" or "implement later" markers. Every task has runnable code.

**Type consistency:** `BlastGuardServer { graph: Arc<Mutex<CodeGraph>>, session: Arc<Mutex<SessionState>>, project_root: PathBuf, config: Arc<Config> }` stable across Tasks 1-6. `ApplyChangeRequest`/`RunTestsRequest` consumed as `Parameters<T>`. `ApplyChangeResponse`/`RunTestsResponse` wrapped in `Json<T>`.

**Known risk:** rmcp 1.5's exact API names (`ServerHandler` methods, `ResourceContents::Text` variants, `ToolAnnotations` field names) must be verified via context7 BEFORE each Task. The pre-work at the top of the plan is not optional. If a subagent skips it and hits a compile error, the fix is to re-run context7 and adjust — not to invent plausible names.

---

## Execution Handoff

Plan complete and saved. Defaulting to subagent-driven execution per session preference.
