# BlastGuard — Technical Specification

## 1. Philosophy

BlastGuard is an open-source Rust MCP server designed to lift AI coding agents on SWE-bench Pro through three mechanisms validated in peer-reviewed and independent-vendor research: AST-based retrieval (cAST paper: up to +2.7 Pass@1 on SWE-bench Lite/Verified — arXiv:2506.15655), semantic search via embeddings (Auggie: +5.9 over the SWE-Agent Scale-AI scaffold on SWE-bench Pro), and tight test-to-code feedback loops (Replay.io: +15 on their own Web Debug Bench — not SWE-bench). Independent corroboration: CodeCompass (arXiv:2602.20048) showed +20pp on hidden-dependency tasks and 0pp on semantic tasks, exactly matching the split BlastGuard's graph-first design predicts. It does not force tool use or gate edits. It offers richer alternatives that agents choose when tasks are complex enough to benefit.

Three documented failure modes drive the design (SWE-bench Pro paper §6.3, profiling Sonnet 4 on the Scale SEAL scaffold):

- **Endless file reading / context overflow** — 62.6% of tasks exhibit endless file reading; narrative attributes ~35.6% of failures to long-context overflow (Table 4 reports 8.7% as a pure token-limit category; the delta is scope-of-analysis). Solved by compact graph queries (50-300 tokens) instead of raw file reads (2000-8000 tokens).
- **Multi-file cascade errors** — SWE-bench Pro averages 4.1 files per reference patch (paper §3). A signature change in file A silently breaks callers in files B, C, D. Solved by warnings in edit responses that list affected callers.
- **Blind iteration on test failures** — Agents often don't know which of their edits caused a test to fail. Solved by mapping stack traces to recently modified functions via the graph.

This spec is MVP-first. Phase 1 ships the core three tools with minimal feature set. Phase 2 adds semantic embeddings and bundled retrieval only if Phase 1 benchmark data supports the effort. Phase 3 is data-driven iteration.

---

## 2. Architecture

```
MCP Server (rmcp, stdio transport)
├── search ─────→ Dispatcher → Graph | Semantic (Phase 2) | Grep
├── apply_change → SymbolDiff → CascadeWarnings → BundledContext → Apply
├── run_tests ──→ RunnerDetect → Execute → Parse → MapToGraph → MapToSession
└── Resource: blastguard://status

CodeGraph ──────→ symbols, forward_edges, reverse_edges, file_symbols, library_imports
VectorIndex ────→ sqlite-vec embeddings (Phase 2)
SessionState ───→ modified_files, modified_symbols, last_test_results
ASTIndexer ─────→ tree-sitter × 4 languages, rayon parallel
GraphCache ─────→ .blastguard/cache.bin via rmp-serde + BLAKE3 Merkle tree
FileWatcher ────→ notify with 100ms debounce, incremental reindex
```

---

## 3. MCP Tools

### 3.1 `search`

**Annotations:** `readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false`

**Description shown to agent:**
```
Search the codebase via AST dependency graph, semantic embeddings (Phase 2),
or regex grep. Structural queries ("callers of X", "tests for FILE") resolve
instantly from the graph with inline signatures. Use "around FILE:symbol" to
retrieve a function plus callers, tests, and related code in one call.
```

**Input:** `{ "query": "...", "scope": null }`

**Query dispatcher:**

| Pattern | Operation | Typical tokens |
|---|---|---|
| `callers of X` / `what calls X` | Reverse BFS with inline signatures | 50-150 |
| `callees of X` / `what does X call` | Forward edges with inline signatures | 50-150 |
| `imports of FILE` / `importers of FILE` | Import edges | 40-120 |
| `exports of FILE` | Symbol list with signatures | 30-100 |
| `chain from X to Y` | BFS shortest path | 40-100 |
| `find X` / `where is X` | Name lookup (exact + Levenshtein ≤ 2) | 30-80 |
| `outline of FILE` | All symbols with signatures | 80-200 |
| `tests for FILE` / `tests for X` | Test files via reverse import edges | 40-120 |
| `around FILE:symbol` (Phase 2) | Symbol + top callers + tests + similar | 200-400 |
| `libraries` | external imports grouped + counts | 60-150 |
| Starts with `semantic:` (Phase 2) | Vector similarity search | 100-300 |
| Anything else | Regex grep via `ignore` + `regex`, cap 30 | 100-400 |

**Results always include inline signatures.** A call to "find processRequest" returns the symbol location plus its full signature in a single string, so the agent rarely needs a follow-up file read.

**Result ranking.** When multiple matches, sort by graph centrality — the count of reverse edges. Symbols with more dependents are typically more relevant than leaf helpers.

### 3.1.1 Semantic search (Phase 2 only)

Generate embeddings for every symbol body at index time using `fastembed` with BGE-small (130MB model, runs locally, no network). Store in `sqlite-vec`. For queries starting with `semantic:` or when graph dispatcher has no match and the query is natural language, compute query embedding and return top-k cosine similarity matches.

Local-only by design. No API calls. Model downloads on first run, cached in `~/.cache/blastguard/`.

Add only if Phase 1 benchmark results support the implementation effort. Academic evidence (Auggie +6, GNN-Coder +20% zero-shot) suggests it's worth the complexity, but validate on your own benchmark first.

### 3.1.2 `around` bundled retrieval (Phase 2 only)

The pattern `around src/handler.ts:processRequest` returns:

```json
{
  "symbol": "processRequest at src/handler.ts:5 — export async function processRequest(req: Request, res: Response): Promise<Response>",
  "body": "... full function body ...",
  "callers": [
    "api.ts:22 — export async function handleApiRequest(req: Request) { processRequest(req, res); }",
    "admin.ts:8 — function handleAdmin(req, res) { processRequest(req, res); }"
  ],
  "tests": ["tests/test_handler.py — test_process_request, test_process_request_invalid"],
  "similar": ["handleAuth at src/auth.ts:12 — similar structure via embedding match"]
}
```

One call replaces a sequence of "find function, read file, grep callers, read callers, find tests, read tests" that would otherwise cost 5-8 tool calls.

### 3.2 `apply_change`

**Annotations:** `readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: false`

**Description shown to agent:**
```
Edit files with impact analysis. Writes immediately — no approval gate. Response
includes warnings about cascade effects (callers that may break, interfaces that
may be violated) and a context bundle (callers, tests, related files) so you
rarely need follow-up searches. Use this for multi-file changes where seeing
blast radius matters. For trivial single-line fixes, your native edit tool is fine.
```

**Input:**
```json
{
  "file": "src/handler.ts",
  "changes": [{ "old_text": "...", "new_text": "..." }],
  "create_file": false,
  "delete_file": false
}
```

**Output:**
```json
{
  "status": "applied",
  "summary": "Modified processRequest() in src/handler.ts. 2 cascade warnings.",
  "warnings": [
    "SIGNATURE: processRequest() changed (req,res)→(req,res,next). 3 callers may break: api.ts:22, admin.ts:8, webhook.ts:15",
    "ASYNC_CHANGE: processRequest() sync→async. Same 3 callers need await"
  ],
  "context": {
    "callers": [
      "api.ts:22 — handleApiRequest(req: Request) { processRequest(req, res); }",
      "admin.ts:8 — handleAdmin(req, res) { processRequest(req, res); }",
      "webhook.ts:15 — onWebhook(event) { processRequest(event.req, event.res); }"
    ],
    "tests": ["tests/test_handler.py"]
  }
}
```

**Key design decisions:**

1. **Writes immediately, no pending/confirm gate.** CodeCompass research shows forcing tool use via gating is unproven. The warnings and context are information for the agent to act on — not approval checkpoints. This eliminates 2 extra turns per edit compared to a gated design.

2. **Warnings, not errors.** Cascade issues are framed as information. The agent decides if they warrant follow-up edits. No `isError: true` for cascade detection.

3. **Context bundle eliminates follow-up searches.** After editing a function, the agent typically wants to update callers. The bundled response pre-fetches them with current code.

4. **Session state updated on every apply.** Used by `run_tests` to attribute failures to recent edits.

### 3.3 `run_tests`

**Annotations:** `readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false`

**Description shown to agent:**
```
Run the project's tests. Auto-detects runner. Returns pass/fail counts and failure
locations mapped back to source functions you recently modified via the graph.
Use after edits. Modern models already self-verify; this tool's unique value is
attribution: linking test failures to your own recent edits.
```

**Input:** `{ "filter": null, "timeout_seconds": 60 }`

**Output:**
```json
{
  "passed": 42,
  "failed": 2,
  "skipped": 1,
  "duration_ms": 3200,
  "failures": [
    "FAIL tests/test_handler.py:23 test_process_request — AssertionError: expected 200 got 500. YOU MODIFIED processRequest() in handler.ts:5 (2 edits ago). Other callers of processRequest: api.ts:22, admin.ts:8",
    "FAIL tests/test_auth.py:45 test_auth_flow — TypeError: missing required argument 'next'. YOU MODIFIED processRequest() in handler.ts:5 (2 edits ago)"
  ]
}
```

**The attribution is the core value.** Parse failures extract stack trace file:line pairs. Resolve against graph symbols. When a failing test's stack trace mentions a symbol in `SessionState.modified_symbols`, append `YOU MODIFIED X in file:line (N edits ago)`. This directly closes the feedback loop documented in the Replay.io research.

**Auto-detection:**

| Project files | Command |
|---|---|
| `package.json` with jest | `npx jest --reporters=default --json` |
| `package.json` with vitest | `npx vitest run --reporter=json` |
| `pytest.ini` / `pyproject.toml` [tool.pytest] / `conftest.py` | `python -m pytest --tb=short -q --json-report` |
| `Cargo.toml` | `cargo test --no-fail-fast -- -Z unstable-options --format json` |
| None detected | `isError: true` with suggestion to use `--test-command` flag |

Parser per runner extracts test name, file:line, error message, and stack trace file:line pairs.

Override via `.blastguard/config.toml` or CLI flag.

### 3.4 MCP Resource

`blastguard://status` returns compact project overview:
```
Index: 4521 files, 23847 symbols, 67234 edges
Languages: TS 3200, JS 800, PY 421, RS 100
Cache: warm, last reindex 3s ago
Semantic index: 23847 vectors, 130MB embedding model loaded (Phase 2)
Test runner: jest detected from package.json
Last test run: 42 pass, 2 fail, 3.2s ago
Session: 3 files modified, no changes pending
Top entry points: src/index.ts, src/api.ts
Most-depended-on: utils/auth.ts (47 dependents), handler.ts (32)
```

The session line helps prevent agent drift on long tasks.

### 3.5 Error Handling

MCP `isError` distinguishes real errors from informational warnings:

```rust
CallToolResult {
  content: [TextContent("ERROR: old_text not found in handler.ts. Closest match at line 12 (92% similar): 'function processRequest(req, res) {' — did you mean this?")],
  is_error: true
}
```

Use `isError: true` only for: file not found, ambiguous `old_text`, parse failure blocking analysis, no test runner detected, test timeout exceeded, test runner crashed.

Cascade warnings in `apply_change` are always `isError: false` — they are information, not errors.

---

## 4. Session State

```rust
pub struct SessionState {
    modified_files: Vec<(PathBuf, Instant)>,
    modified_symbols: Vec<(SymbolId, Instant)>,
    last_test_results: Option<TestResults>,
    session_start: Instant,
}
```

Updated on every successful edit and test run. Used by `run_tests` to attribute failures, `blastguard://status` for session summary, and `apply_change` bundled response to include `recently_modified_nearby` context.

In-memory only. Resets on server restart. This aligns with per-task evaluation environments like SWE-bench's containerized runs.

---

## 5. Cascade Analysis

### 5.1 Symbol Diffing

Parse full old file and proposed new file with tree-sitter. Build symbol tables keyed by `(name, kind)`. Diff produces:

- **added** — in new, not old
- **removed** — in old, not new
- **modified-sig** — in both, signature differs
- **modified-body** — in both, body hash differs, signature matches

Zero changes across all categories (whitespace/comments only) → apply without warnings.

### 5.2 Four Cascade Warnings (Phase 1)

Research guidance: avoid over-reporting. CodeCompass found agents ignore complex/noisy tool outputs. Ship with the four highest-signal checks. Add more based on Phase 1 instrumentation data.

1. **SIGNATURE** — Modified symbol, param count/types/return type changed. Lists all callers with their current call sites. Highest signal — directly indicates cascade failure.

2. **ASYNC_CHANGE** — Function changed between sync and async. Callers that don't `await` get a Promise instead of a value. Pure AST check on `async` keyword presence.

3. **ORPHAN** — Symbol removed or file deleted. Remaining callers will break. Straightforward reverse-edge lookup.

4. **INTERFACE_BREAK** — TypeScript interface or Rust trait modified. Implementing classes/structs may no longer satisfy the contract. Tracked via `EdgeKind::Implements`.

### 5.3 Phase 2 Cascade Warnings (data-driven)

Add based on Phase 1 instrumentation. Candidates, in order of likely signal:

5. **PARAM_ORDER** — Same count, reordered names. Deadly in positional-arg languages.
6. **VISIBILITY** — Exported → private, or public → internal.
7. **REEXPORT_CHAIN** — Modified symbol re-exported through barrel file. Follow `EdgeKind::ReExports`.
8. **CIRCULAR_DEP** — New import creates cycle.

Add only if Phase 1 shows they fire on ≥ 5% of SWE-bench Pro tasks with ≥ 70% precision.

### 5.4 Output Rules

Each warning is a single string under 200 characters. Cap at 10 callers per warning with `"...and N more (M total)"`. One-line summary at response top: `"3 warnings: 1 SIGNATURE, 1 ASYNC_CHANGE, 1 ORPHAN"`. This lets the agent skip details if summary is clean.

---

## 6. Import Path Resolution

### 6.1 TypeScript / JavaScript
`from './utils/auth'` → try `.ts`, `.tsx`, `.js`, `.jsx`, `/index.ts`, `/index.tsx`, `/index.js`. External → `library_imports`.

### 6.2 Python
`from utils.auth import x` → `utils/auth.py` → `utils/auth/__init__.py`. Relative from package dir. External → `library_imports`.

### 6.3 Rust
`use crate::utils::auth` → `src/utils/auth.rs` → `src/utils/auth/mod.rs`. External → `library_imports`.

### 6.4 tsconfig.json Path Aliases
Parse `tsconfig.json` at startup. Extract `compilerOptions.paths` and `baseUrl`. Build prefix map. Apply before relative fallback.

### 6.5 Implementation
```rust
pub enum ResolveResult {
    Internal(PathBuf),
    External { library: String, symbols: Vec<String> },
    Unresolved,
}
```

Unresolved imports produce edges with `Confidence::Inferred` rather than being dropped. Visible to the agent with appropriate caveats.

---

## 7. Data Structures

```rust
pub struct SymbolId { file: PathBuf, name: String, kind: SymbolKind }

pub enum SymbolKind {
    Function, AsyncFunction, Class, Method, Interface,
    TypeAlias, Constant, Export, Module, Trait, Struct,
}

pub struct Symbol {
    id: SymbolId,
    line_start: u32,
    line_end: u32,
    signature: String,
    params: Vec<String>,
    return_type: Option<String>,
    visibility: Visibility,
    body_hash: u64,
    is_async: bool,
    embedding_id: Option<i64>,  // Phase 2 only, row ID in sqlite-vec
}

pub enum Visibility { Export, Public, Private, Default }

pub struct Edge {
    from: SymbolId,
    to: SymbolId,
    kind: EdgeKind,
    line: u32,
    confidence: Confidence,
}

pub enum EdgeKind {
    Calls, Imports, Inherits, Implements,
    Exports, ReExports, TypeReference,
}

pub enum Confidence { Certain, Inferred }

pub struct LibraryImport {
    library: String,
    symbol: String,
    file: PathBuf,
    line: u32,
}

pub struct CodeGraph {
    symbols: HashMap<SymbolId, Symbol>,
    forward_edges: HashMap<SymbolId, Vec<Edge>>,
    reverse_edges: HashMap<SymbolId, Vec<Edge>>,
    file_symbols: HashMap<PathBuf, Vec<SymbolId>>,
    library_imports: Vec<LibraryImport>,
    centrality: HashMap<SymbolId, u32>,
}
```

Both edge maps always maintained. Centrality cached as in-degree count. No `PendingStore` — design decision removed in this iteration because forced approval gates are unproven and add 2 turns per edit.

---

## 8. Tree-sitter

### 8.1 Extension Map
`.ts .tsx .mts .cts` → typescript | `.js .jsx .mjs .cjs` → javascript | `.py .pyi` → python | `.rs` → rust

Go deferred to Phase 3 pending data on Go task performance.

### 8.2 Extraction Per File
Functions (name, params, return type, lines, body hash, is_async, visibility), classes/structs/interfaces/traits (name, extends/implements, members), imports (internal + external), exports + re-exports, call sites (callee + containing function).

### 8.3 Query Files
`queries/{language}.scm`. JS shares most TS queries minus type annotations.

### 8.4 Body Hash Normalization
Strip whitespace per line. Collapse blank lines. Remove `//`, `#`, and `/* */` comments. Seahash. Stable across formatting changes.

### 8.5 Graceful Degradation
On parse errors: extract what parsed successfully, mark file `partial_parse = true`, continue. Agent sees partial results with explicit note rather than a crash.

---

## 9. Graph Cache

`.blastguard/cache.bin` via `rmp-serde`:

```rust
pub struct CacheFile {
    version: u32,
    file_hashes: HashMap<PathBuf, u64>,        // BLAKE3 of file content
    tree_hashes: HashMap<PathBuf, u64>,        // BLAKE3 of parent directory content
    graph: CodeGraph,
    tsconfig: Option<TsConfig>,
}
```

Warm start algorithm: load cache, compute current hashes in parallel via rayon, skip entire unchanged subtrees via tree_hashes, re-parse only changed files. Targets sub-500ms warm start on 10K file projects.

Phase 2: include embedding index state in cache invalidation.

Invalidation triggers: BlastGuard version mismatch, cache file corruption, tsconfig.json changed.

---

## 10. Startup and Performance

1. Determine project root from CLI argument or cwd
2. Load `.blastguard/config.toml` if present
3. Load cache if version matches
4. Parse `tsconfig.json` for path aliases
5. Detect test runner from project files
6. Walk files via `ignore` crate (respects .gitignore)
7. Hash files and directory trees via BLAKE3 in parallel
8. Parse changed/new files via `rayon` with one tree-sitter parser per worker
9. (Phase 2) Generate embeddings for new/modified symbols
10. Save cache
11. Start `notify` watcher at 100ms debounce
12. Initialize SessionState
13. Begin MCP handshake. Non-blocking — advertise tools immediately, emit progress notifications during ongoing work.

**Performance targets:**

| Operation | Target |
|---|---|
| Cold index, 10K files | under 3 seconds |
| Warm start | under 500ms |
| Single file reindex | under 50ms |
| Cascade analysis | under 10ms |
| Structural search | under 100ms |
| Grep search | under 300ms for full repo |
| Semantic search (Phase 2) | under 200ms for top 10 |
| Memory footprint | ~200MB per 50K files + ~130MB embeddings (Phase 2) |

---

## 11. File Watcher

`notify` + `notify-debouncer-mini` at 100ms. On change: re-parse, update symbols, diff old vs new, update forward/reverse edges incrementally, update library_imports, regenerate embeddings for changed symbols (Phase 2). On delete: remove all symbols and edges for the file. On create: parse and add. Ignore files matching `.gitignore`.

Runs in dedicated Tokio task, updates graph under write lock.

---

## 12. Dependencies

```toml
[dependencies]
# Verified against crates.io on 2026-04-18 via context7 MCP.
rmcp = { version = "1.5", features = ["server", "transport-io", "macros", "schemars"] }
schemars = "0.9"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tree-sitter = "0.24"                 # 0.26.x available; hold unless grammar regressions surface
tree-sitter-typescript = "0.23"
tree-sitter-javascript = "0.23"
tree-sitter-python = "0.23"
tree-sitter-rust = "0.21"               # 0.22+ emits ABI 15; tree-sitter 0.24 accepts max ABI 14 — upgrade tree-sitter core to 0.26+ (post-MVP) to use 0.24.
notify = "8"
notify-debouncer-mini = "0.7"        # must track notify major version
ignore = "0.4"
regex = "1"
rayon = "1.10"
rmp-serde = "1"
blake3 = "1"
strsim = "0.11"
seahash = "4"
uuid = { version = "1", features = ["v4"] }
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
toml = "0.8"

# Phase 2 additions (feature-flagged)
sqlite-vec = { version = "0.1", optional = true }
rusqlite = { version = "0.32", optional = true }
fastembed = { version = "5", optional = true }    # v5 Candle backend; ONNX path still supported

[features]
default = []
semantic = ["sqlite-vec", "rusqlite", "fastembed"]

[dev-dependencies]
tempfile = "3"
pretty_assertions = "1"
criterion = "0.5"

[profile.release]
lto = true
codegen-units = 1
strip = true
```

Semantic search is a cargo feature flag. Default build is Phase 1 only — small binary, no ML model download. `--features semantic` enables Phase 2 capabilities.

---

## 13. Edge Cases

| Case | Handling |
|---|---|
| `old_text` not found | `isError: true` with closest fuzzy match and similarity % |
| `old_text` ambiguous | `isError: true` with match count and line numbers |
| Parse error during apply | Mark file partial, continue with what parsed, include note |
| Unsupported extension | Index for grep only, skip AST |
| Binary / >1MB file | Skip AST, available for grep with warning |
| Concurrent tool calls | Tokio Mutex on graph write, RwLock for search |
| Agent calls during indexing | Serve what's ready, progress notifications, never block |
| External file change during work | Watcher updates graph |
| CRLF line endings | Normalize to LF for storage and comparison |
| Dynamic dispatch (`getattr`, `obj[method]()`) | Mark `Confidence::Inferred`, show agent |
| Grep > 30 results | Return top 30, suggest narrowing query |
| No test runner detected | `isError: true` with `--test-command` suggestion |
| Test timeout | Kill process, return partial results with timeout note |
| Test runner crashes | `isError: true` with stderr (truncated to 500 chars) |
| Semantic model download fails | Fall back to graph + grep only, warn at startup |

---

## 14. Project Structure

```
blastguard/
├── Cargo.toml
├── README.md                  # Honest positioning, comparison table, limitations
├── LICENSE                    # MIT
├── CHANGELOG.md
├── queries/
│   ├── typescript.scm
│   ├── javascript.scm
│   ├── python.scm
│   └── rust.scm
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── config.rs
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   └── tools.rs
│   ├── search/
│   │   ├── mod.rs
│   │   ├── dispatcher.rs
│   │   ├── structural.rs
│   │   └── text.rs
│   ├── semantic/              # feature = "semantic"
│   │   ├── mod.rs
│   │   ├── embed.rs
│   │   └── store.rs
│   ├── analysis/
│   │   ├── mod.rs
│   │   ├── graph.rs
│   │   └── impact.rs
│   ├── parser/
│   │   ├── mod.rs
│   │   ├── indexer.rs
│   │   ├── symbols.rs
│   │   └── resolve.rs
│   ├── runner/
│   │   ├── mod.rs
│   │   ├── detect.rs
│   │   ├── execute.rs
│   │   └── parse.rs
│   ├── session.rs
│   ├── cache.rs
│   └── watcher.rs
├── tests/
│   ├── integration.rs
│   └── fixtures/
│       └── sample_project/
└── bench/                     # benchmark harness (SPEC §15)
    ├── README.md
    ├── harness/
    │   └── swebench_runner.ts  # based on mini-SWE-agent v2
    ├── run_baseline.sh
    ├── run_with_blastguard.sh
    └── compare.py
```

---

## 15. Benchmark Harness (required for MVP)

A first-class deliverable. Not optional. Without measurement, the spec's thesis is untested.

### 15.1 Requirements

- Runs SWE-bench Pro public set (731 tasks) with and without BlastGuard
- Uses mini-SWE-agent v2 or equivalent as base scaffold to isolate BlastGuard's contribution
- Supports both Opus 4.7 and GLM-5.1 (cheap model sanity check)
- Produces per-task JSONL with: task_id, resolved, turns_used, tokens_in, tokens_out, tool_calls_per_type, wall_time
- Generates comparison report: delta resolution rate, delta tokens, delta turns, per-repo breakdown

### 15.2 Instrumentation

Every tool call records: tool name, input size, output size, wall time. Every cascade warning records: type, whether agent followed up. Every graph query records: cache hit, latency.

### 15.3 Published Results

Ship in README.md with full methodology, confidence intervals, and trajectories. Compare against:
- Baseline (no BlastGuard, same scaffold)
- code-graph-mcp (closest open-source competitor — Rust + BLAKE3 Merkle; no published SWE-bench number as of 2026-04-18)
- WarpGrep v2 (Morph, self-reported +2.1pt on Opus 4.6, +3.7pt on MiniMax 2.5 on SWE-bench Pro)
- Auggie (Augment Code, self-reported +5.9 over SWE-Agent Scale-AI scaffold on SWE-bench Pro)

Be honest. If BlastGuard shows 0 or negative lift, publish that. If +1pt with ±2 confidence interval, say so. The goal is evidence, not marketing.

### 15.4 Grading isolation (Berkeley BenchJack defense)

UC Berkeley RDI published a benchmark-exploit paper (April 2026) showing SWE-bench tasks can be trivially gamed by an agent writing a 10-line `conftest.py` that forces pytest to report all tests as passing — 45 confirmed exploits across 8 benchmarks. The BlastGuard harness MUST:

- Run grading in a separate process/container with its own pristine pytest config.
- Ignore any `conftest.py`, `pytest.ini`, or `pyproject.toml` overrides the agent creates inside the task directory during grading.
- Diff agent-touched files before grading; flag any `.git`, `conftest.py`, or CI config modifications as a tampering failure (counts as unresolved).
- Never execute arbitrary agent-written Python in the grader's process.

This is a non-negotiable requirement. Without it, our own benchmark numbers cannot be trusted.

---

## 16. Testing

**Unit:** graph operations (BFS, DFS, shortest path), each cascade check individually, symbol diffing, query dispatcher for every pattern, import resolution × 4 languages, tsconfig aliases, fuzzy matching, test output parsing per runner, failure-to-graph mapping, session state, centrality, body hash normalization, graceful parse degradation.

**Integration:** full MCP flow for all 3 tools + resource. Search: every dispatcher pattern, grep fallback, regex, scope. Apply: each cascade warning firing correctly, trivial auto-apply, file creation, file deletion, context bundle accuracy. Run_tests: auto-detection per runner, filter, timeout behavior, attribution via graph, stderr on crash.

**Benchmark-as-test:** A minimal subset of 10 SWE-bench Pro tasks runs as part of `cargo test` to detect regressions. Full benchmark runs separately via `cd bench && ./run_with_blastguard.sh`.

**Fixtures:** Multi-file TypeScript project with cross-file calls, class implementing interface, async function, barrel re-export, external library (lodash with only `get` used), file with syntax error for degradation testing, `tsconfig.json` with path alias, `package.json` declaring jest, test file with 2 passing and 2 failing tests referencing functions modified during integration testing.

---

## 17. Distribution

Single binary. `cargo build --release` with LTO and strip. MIT licensed.

```bash
cargo install blastguard                    # Phase 1 only, ~8MB binary
cargo install blastguard --features semantic # Phase 2, ~140MB with model

claude mcp add blastguard -- blastguard /path/to/project
```

README.md must include:
- Honest positioning (not a silver bullet)
- Measured benchmark lift with confidence intervals
- Comparison table: vexp (commercial), code-graph-mcp (open), WarpGrep (closed), BlastGuard
- Known limitations (dynamic dispatch blind spots, Go unsupported in Phase 1, etc.)

---

## 18. Decision Log

Design decisions explicitly made based on research evidence:

| Decision | Rationale |
|---|---|
| No pending/confirm gating | CodeCompass: forced tool use unproven; 2 extra turns per edit is a real cost |
| Start with 4 cascade checks | CodeCompass: agents ignore noisy outputs; expand based on data |
| Semantic search behind feature flag | Auggie: +6pt evidence strong, but adds 130MB binary and model download latency |
| Drop Go initially | 4 languages already cover ~80% of SWE-bench Pro; add based on failure distribution |
| Drop session-state deduplication | Speculative complexity; replacing shown content with placeholders risks hurting more than helping |
| Benchmark harness in MVP | Without measurement, every claim is unfounded |
| Honest README | Research shows benchmark hacking is rampant; integrity is differentiation |
