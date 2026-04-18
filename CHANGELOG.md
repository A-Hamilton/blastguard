# Changelog

All notable changes to BlastGuard are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — Phase 1 MVP (unreleased)

### Added

- **Code graph** (Phase 1.1): `Symbol`, `Edge`, `CodeGraph` with forward + reverse
  adjacency maps, centrality cache, `remove_file` preserving caller forward edges
  for ORPHAN cascade detection. BFS/DFS helpers — `callers`, `callees`,
  `shortest_path`, `find_by_name` with exact + Levenshtein ≤ 2 fallback.
- **Language drivers** (Phase 1.2): TypeScript, JavaScript, Python, Rust via
  tree-sitter. Emits symbols, library imports, internal `Imports` edges marked
  `Confidence::Unresolved`, and intra-file `Calls` edges with per-extract dedup.
  Arrow functions are transparent to call attribution. Graceful partial-parse on
  tree-sitter `ERROR` nodes.
- **Import resolvers** (Phase 1.3): TS relative + `tsconfig.json` path aliases
  (JSONC comments stripped); Python dotted modules; Rust `crate::` / `self::` /
  `super::`.
- **Parallel indexer + BLAKE3 Merkle cache** (Phase 1.4): `cold_index` via `ignore`
  walker + `rayon` fan-out. `hash_project_tree` derives the Merkle root from the
  gitignore-filtered file set, so `node_modules/` churn doesn't invalidate the
  fast-path. Warm start loads cache, runs parallel BLAKE3 diff, reparses only
  changed files; survives file-disappeared races. Disk persistence via `rmp-serde`
  with version gate.
- **`search` tool** (Phase 1.5): query classifier + 10 structural arms
  (`callers of`, `callees of`, `outline of`, `chain from…to`, `find` / `where is`,
  `tests for`, `imports of`, `importers of`, `exports of`, `libraries`) + regex
  grep fallback capped at 30 hits. Centrality-ranked results.
- **`apply_change` tool** (Phase 1.6): symbol diff (added / removed /
  modified-sig / modified-body); four cascade detectors (SIGNATURE, ASYNC_CHANGE,
  ORPHAN, INTERFACE_BREAK) with callers / implementors listed; bundled context
  via `search::structural::callers_of_id` + `tests_for`. Multi-change edits roll
  back to the original file on mid-sequence failure. `create_file` refuses to
  overwrite existing files. `EditNotFound` carries closest-match hint + similarity;
  `AmbiguousEdit` lists every match line number.
- **`run_tests` tool** (Phase 1.7): auto-detects jest / vitest / pytest / cargo
  from project files. Spawns with timeout (kills on overrun). Parsers extract
  per-test failure file:line + stack. Attribution appends
  `YOU MODIFIED X (N edits ago)` when a stack frame lands inside a symbol the
  session has edited. Non-zero exit + zero parsed counts surfaces as
  `TestCrashed` with truncated stderr.
- **rmcp 1.5 MCP stdio server** (Phase 1.8): `BlastGuardServer` with
  `#[tool_router]` + three `#[tool]` handlers wrapping the backends. Handlers
  are async and wrap blocking work in `tokio::task::spawn_blocking` so the
  runtime never stalls. `blastguard://status` resource.
- **File watcher** (Phase 1.9): `notify-debouncer-mini` at 100ms debounce,
  gitignore-aware, filters non-source extensions. Runs on a dedicated tokio
  task with a named relay thread bridging the debouncer's std-mpsc handler.
  Degrades gracefully on poisoned graph lock. Shuts down cleanly within 200ms
  of abort.
- **Benchmark harness** (Phase 1.10): Python 3.11 + uv. `bench/runner.py`
  spawns BlastGuard as an MCP subprocess under a minimal tool-use agent loop
  (Anthropic + OpenAI-compatible providers). `bench/grader.py` defends against
  the Berkeley BenchJack `conftest.py` exploit by flagging any change to
  `conftest.py` / `pytest.ini` / `tox.ini` / `.github/workflows/**` as
  tampering. `bench/compare.py` emits resolution-rate / tokens / turns deltas
  per repo.

### Quality

- 251 Rust library tests + 7 integration tests + 11 Python harness tests,
  all passing.
- `cargo clippy --all-targets -- -W clippy::pedantic -D warnings` clean.
- `ruff check` clean on the Python harness.
- Single binary approximately 8-9 MB (LTO + strip). Exact size drifts with
  toolchain version.

### Known limitations

- Cross-file call edges aren't resolved yet (SPEC §6 — Phase 2).
- Go language support is deferred pending benchmark evidence.
- Semantic search (`around X` + embeddings) is a feature-flagged Phase 2 item.
- `cargo test -- --format json` requires nightly (`-Z unstable-options`); the
  harness surfaces stable-toolchain runs as `TestCrashed` rather than silently
  returning zero counts.
