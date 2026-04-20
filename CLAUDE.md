# BlastGuard — Build Instructions

Read `SPEC.md` completely before writing code. This is an evidence-backed, MVP-first spec. No hype. No over-engineering.

## What This Is

Open-source Rust MCP server (MIT). Three tools designed to lift AI coding agents on SWE-bench Pro through better retrieval, tighter test feedback, and cascade warnings.

- `search` — AST graph queries + semantic search via embeddings + regex grep fallback. Includes `around X` bundled retrieval.
- `apply_change` — Rich edit tool. Writes immediately. Response includes cascade warnings, callers, tests, and related files in one bundle.
- `run_tests` — Auto-detects runner. Maps failures back to recently modified functions via the graph.

BlastGuard does not replace or gate the agent's native tools. It offers richer alternatives the agent can choose when useful.

## Hard Rules (Do Not)

- **Do not edit `Cargo.toml` by hand** for dependencies. Use `cargo add <crate>` / `cargo add --dev <crate>`. Hand-edit only for `[features]`, `[profile.*]`, workspace config.
- **Do not touch `Cargo.lock`** manually. Let `cargo` regenerate it.
- **Do not invent crate APIs.** `rmcp`, `tree-sitter`, `sqlite-vec`, `fastembed` all evolve fast — validate via `mcp__context7` before writing against them. Your training memory is stale.
- **Do not use `println!` / `eprintln!` / `print!`** in any code path. Use `tracing::{debug,info,warn,error}!`. The MCP stdio transport owns stdout; stdout writes break the protocol.
- **Do not use `.unwrap()` / `.expect()`** outside tests, build scripts, or genuinely unreachable branches documented with a comment explaining why.
- **Do not use `panic!` / `todo!` / `unimplemented!`** in committed code. Return an error.
- **Do not swallow errors** with `let _ = result;` or `.ok()` silently. Log at `warn` or propagate.
- **Do not spawn blocking work on the Tokio runtime** — use `tokio::task::spawn_blocking` or `rayon` for CPU work (parsing, BLAKE3, embeddings).
- **Do not add a dependency** without checking it's actively maintained and the version is in SPEC.md's verified list. New deps need a line in CLAUDE.md + Cargo.toml comment explaining why.
- **Do not create README / docs files** unless asked. `SPEC.md`, code, and commit messages are the record.
- **Do not use `pub` by default.** Start with `pub(crate)` and narrow where possible. Only promote to `pub` at the crate's public API surface.
- **Do not write integration tests that shell out to the real benchmark.** Keep unit tests fast (<5s total). Benchmark harness is opt-in via a separate binary or `--ignored` test.
- **Do not over-abstract.** Three similar match arms is fine — premature traits hurt more than they help. Three concrete impls first, trait extraction when a fourth case forces it.
- **Do not add features beyond the current phase.** Phase 2 work is blocked until Phase 1 ships benchmark numbers.

## Evidence-Backed Performance Target

Realistic projection based on peer-reviewed research:
- **cAST (AST-based RAG chunking):** +2.67 Pass@1 on SWE-bench
- **WarpGrep v2 (RL-trained search subagent):** +2.1-2.2 on SWE-bench Pro
- **Auggie (semantic retrieval):** +6 over SEAL baseline
- **Replay.io debugging MCP:** +15 on their benchmark (different task type)

Projected lift on SWE-bench Pro: **+1 to +3 points** with realistic confidence interval of **-1 to +5**. Target state-of-the-art combinations:
- Opus 4.7 + BlastGuard: 65% to 67% (from 64.3% baseline)
- GLM-5.1 + BlastGuard: 60% to 63% (from 58.4% baseline)

## Stack (verified April 2026 — reconciled against Cargo.toml)

- `rmcp` v1.5 with `["server", "transport-io", "macros", "schemars"]` + `schemars` v1
- `tree-sitter` v0.24 + grammars: TS v0.23, JS v0.23, Python v0.23, Rust **v0.21** (0.22+ emits ABI 15; core 0.24 max ABI is 14 — upgrading core to 0.26+ is post-MVP)
- `streaming-iterator` v0.1.9 (required by tree-sitter 0.24's `QueryCursor::matches` streaming API)
- `sqlite-vec` v0.1.6+ for semantic vector search (Phase 2, feature-flagged)
- `fastembed` v5 for local embeddings (BGE-small or similar, ~130MB; Phase 2, feature-flagged)
- `notify` v8 + `notify-debouncer-mini` v0.7 (must track notify major version)
- `ignore` v0.4, `regex` v1, `rayon` v1.10, `rmp-serde` v1
- `blake3` v1, `strsim` v0.11, `seahash` v4, `uuid` v1, `toml` v0.8
- `tokio` v1, `serde` v1, `thiserror` v2, `anyhow` v1, `tracing` v0.1

Go support deferred to post-MVP. Add after data shows Go tasks are where we lose points.

## Build Order (MVP-first)

Run `cargo build && cargo clippy -- -W clippy::pedantic` after each phase. Zero warnings to proceed.

**Phase 1 — MVP (ship to benchmark):**
1. Scaffold + data structures + graph ops (SPEC §7)
2. Tree-sitter parsers TS/JS/PY/RS (SPEC §8)
3. Import resolver + tsconfig paths (SPEC §6)
4. Parallel indexer + rmp-serde cache with BLAKE3 Merkle (SPEC §9-10)
5. `search` with graph dispatcher + regex grep (SPEC §3.1, skip semantic initially)
6. `apply_change` with 4 cascade warnings (SPEC §5 — only SIGNATURE, ASYNC_CHANGE, ORPHAN, INTERFACE_BREAK)
7. `run_tests` with failure-to-code mapping (SPEC §3.3)
8. MCP server layer + `isError` handling (SPEC §3.4)
9. File watcher (SPEC §11)
10. Benchmark harness (SPEC §15)
11. **RUN SWE-BENCH PRO PUBLIC SET AT THIS POINT.** Commit results.

**Phase 2 — Evidence-backed expansion (only if MVP data supports it):**
12. Semantic search via sqlite-vec + fastembed (SPEC §3.1.2)
13. `around X` bundled retrieval pattern (SPEC §3.1.3)
14. Additional cascade checks if data shows they fire usefully: PARAM_ORDER, VISIBILITY, REEXPORT_CHAIN, CIRCULAR_DEP
15. Go language support if Go tasks are losing points

**Phase 3 — Polish based on benchmark diff:**
16. Whatever the benchmark data points to

## Rust Quality

No `.unwrap()` / `.expect()` in production paths — return `Result` with `thiserror` error types, propagate with `?`. `///` on all public items. `#[must_use]` on `Result`-returning functions and constructors. `&str` over `String`, `&[T]` over `Vec<T>` for params. Per-module `#[cfg(test)] mod tests`. `tracing` to **stderr only** — MCP stdio transport uses stdout for protocol frames, so `println!` or stdout logging will corrupt the channel. No `dbg!` in committed code. Prefer `anyhow::Result` at binary boundaries, `thiserror` enums in library modules.

## Role & Working Style

Senior Rust engineer building an MCP server. Direct, assume competence, skip pleasantries. Autonomous on reversible local actions (edits, `cargo check`, `cargo test`). Confirm before anything destructive (`git reset --hard`, force-push, deleting the index cache, editing `Cargo.lock` by hand).

**Decision discipline:** state the chosen approach in one line before acting. When the user is wrong, push back concisely with evidence. If two paths exist and the choice matters, give both in 1–2 sentences — don't pad.

**Model policy:** main session uses `opusplan` — Opus 4.7 for planning/architecture, auto-switches to Sonnet 4.6 for execution. Subagents run Sonnet by default. Escalate with `/effort high` or `xhigh` when stuck on hard graph/concurrency reasoning.

## Preferred Patterns

- **Errors:** `thiserror` in library modules with named variants carrying context; `anyhow` only at the binary entry point. Every `?` crosses a well-typed boundary.
- **Concurrency:** `tokio` for I/O (MCP stdio, file watcher), `rayon` for CPU parallelism (parser fan-out, BLAKE3 hashing). Don't mix — hand off with channels or `spawn_blocking`.
- **Serialization:** `rmp-serde` for the on-disk cache (already in stack), `serde_json` only for the MCP wire protocol via `rmcp`.
- **Graph storage:** in-memory `HashMap`/`FxHashMap` during indexing, `rmp-serde` snapshot on disk with BLAKE3 Merkle root for warm-start validation (SPEC §10).
- **Parsing:** one `tree-sitter::Parser` per thread (they're not `Send`-friendly across `.await`). Create fresh per `rayon` worker.
- **Tests:** `#[cfg(test)] mod tests` colocated, `proptest` for graph invariants, `insta` snapshots for cascade-warning output (SPEC §5). Fixtures under `tests/fixtures/`.
- **Logging:** `tracing` with structured fields (`tracing::info!(file = %path, nodes = count, "indexed")`). Init a `tracing_subscriber` that writes to stderr only, JSON format in release, pretty in debug.
- **Feature gates:** use Cargo features for optional capabilities (`semantic` for sqlite-vec + fastembed) so Phase 1 MVP ships without the ~130MB embedding model.

## File Conventions

- Rust modules: `snake_case.rs`, one logical unit per file.
- Types: `PascalCase`. Functions/methods: `snake_case`. Constants: `SCREAMING_SNAKE_CASE`.
- `src/main.rs` — binary entry, `tracing` init, `rmcp` server wire-up only.
- `src/mcp/` — tool handlers (`search.rs`, `apply_change.rs`, `run_tests.rs`) and `isError` mapping.
- `src/graph/` — node/edge types, graph ops, cascade checks.
- `src/parse/` — tree-sitter drivers per language.
- `src/index/` — parallel indexer, BLAKE3 Merkle, rmp-serde cache, file watcher.
- `src/search/` — graph dispatcher, regex grep, (Phase 2) semantic.
- `src/bench/` — benchmark harness (separate binary target).
- `tests/` — integration tests and fixtures.
- Co-locate types in the same file as the function that owns them unless shared by ≥2 modules.

## MCP Servers (available at user scope)

Only one is routinely useful for this project:

- **context7** (`@upstash/context7-mcp`) — pull up-to-date docs for any Rust crate in the stack. Call BEFORE writing against `rmcp`, `tree-sitter`, `sqlite-vec`, `fastembed`, `tokio`, `notify`, `rmp-serde`, or any crate whose API may have changed since your training cutoff. Use `mcp__context7__resolve-library-id` then `mcp__context7__query-docs`. This is cheaper than guessing and re-compiling.

Ignore the Shopify/Astro/Figma/Stitch/Playwright/Vercel/Ahrefs MCPs — not applicable to a Rust stdio MCP server. `claude-in-chrome` has no use here either.

## Superpowers Skills — When They Fire

The non-negotiables hook lists these. They apply to this project as follows:

- **superpowers:brainstorming** — invoke BEFORE building any new feature, tool handler, or cascade check. Don't skip even for "obvious" work; the discipline catches premature commitment. Skip only for edits to docs/config or trivial bug fixes.
- **superpowers:systematic-debugging** — invoke BEFORE proposing any fix for a failing `cargo test`, clippy regression, MCP protocol error, or wrong cascade-warning output. Don't guess — reproduce, bisect, form a hypothesis, verify.
- **superpowers:writing-plans** — invoke for any change spanning ≥3 files or introducing a new module. Save the plan in-session; don't write plan documents unless the user asks.
- **superpowers:executing-plans** — pair with the above for multi-phase implementation.
- **superpowers:verification-before-completion** — MANDATORY before any "done/fixed/passing" claim. See the verification section below for the exact commands.
- **superpowers:requesting-code-review** — run before any commit the user asks for.

The global non-negotiables mention `pnpm check && pnpm build` — that's for the user's Astro projects. **For BlastGuard, substitute `cargo check && cargo build && cargo test && cargo clippy -- -W clippy::pedantic`.** The intent (verify before claiming done) is identical.

The other global non-negotiables (UI screenshots, Shopify GraphQL validation, Astro/React/Tailwind checks, React island rules) **do not apply** to this repo.

## Subagent Routing

Delegate actively — keep verbose search/research output out of the main context.

| Task | Subagent | Why |
|---|---|---|
| Find a symbol / trace call paths / answer "where does X live" | **Explore** (built-in, Haiku) | Fast and cheap; disposable context. |
| Design a multi-file feature or architecture decision | **Plan** (built-in) | Structured design output. |
| Review implementation before commit | **code-reviewer** | Catches issues before the user sees "done". |
| Research a crate, algorithm, or paper (cAST, WarpGrep, Auggie) | **researcher** | Returns condensed findings, not raw HTML. |
| Open-ended multi-step research across the repo | **general-purpose** | When a single tool won't do. |
| Independent parallel work (e.g. "add parser X and wire tool Y") | split across two agents, run in parallel | Keep them non-overlapping. |

Ignore: `astro-expert`, `shopify-headless`, `react-islands`, `ui-design`, `stitch-designer`, `copywriter`, `analyst`, `scraper`, `vercel:*`, `audit-*` — all UI/marketing/adtech, not Rust.

Prefer **direct tools** (Grep, Glob, Read) over subagents when the target is already known — a subagent adds latency. Use Explore when the search might span >3 queries or naming conventions are uncertain.

## Workflow

1. **Plan first** for anything non-trivial (≥3 files, new tool, new cascade check, indexer change). One paragraph or a bullet list is enough — no plan documents unless asked.
2. **Implement with tests.** Write the test that pins the behavior first when the behavior is nontrivial (graph ops, cascade logic, import resolution). Don't TDD getters.
3. **Verify locally** after every meaningful change: `cargo check` continuously, `cargo clippy -- -W clippy::pedantic` before declaring a phase done.
4. **Review** via `code-reviewer` subagent before claiming done.
5. **Commit only when asked.** Messages: imperative mood, why > what, ≤72-char subject, body paragraph when the why needs explaining. Do not co-author with tool agents unless asked.

## Verification Before "Done"

Before claiming a feature, fix, or phase is complete, run and confirm success:

```bash
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -W clippy::pedantic -D warnings
cargo build --release
```

All four must pass with zero warnings before saying "done". If tests are slow (>30s), scope to the touched module: `cargo test -p blastguard graph::`. The full suite still runs before commit.

Benchmark harness runs are a separate, opt-in verification gate at the end of Phase 1 — not on every change.

## Git & Commit Discipline

- Commit only when the user asks.
- Never `--no-verify` or skip hooks without explicit permission.
- Prefer a new commit over `--amend` once something is pushed.
- Never force-push to `main`.
- Before destructive ops (`git reset --hard`, `git clean -fd`, deleting the index cache directory), confirm with the user.

## Token Efficiency

- Use Grep/Glob, never shell `grep -r` or `find`.
- `rg` over `grep` when going to Bash anyway.
- Read files fully once; don't re-read unchanged files.
- Delegate broad exploration to the Explore subagent — its context is disposable.
- `git diff <path>` on specific files, not repo-wide diffs.
- Don't paste large file contents in summaries — reference `src/graph/ops.rs:142` instead.
- Unknown file type? `npx magika <file>` before guessing.

## Hook-Enforced Rules (global)

Two global hooks run on every project:

- **PostToolUse on Edit|Write** — blocks JS/TS/Astro files containing `console.log`. Not applicable to `.rs` files, but the equivalent project rule stands: no `println!` / `eprintln!` in committed Rust code.
- **PreToolUse on Bash** — blocks catastrophic patterns (`rm -rf /`, `mkfs.*`, `dd of=/dev/...`, force-push to `main`).

If a hook blocks a command, fix the underlying action — don't rewrite the hook.

## Definition of Done (MVP)

- [ ] Single binary, four languages (TS/JS/Python/Rust), MCP over stdio
- [ ] `search`: graph queries + regex grep with inline signatures
- [ ] `apply_change`: writes immediately, returns bundled context + 4 cascade warnings
- [ ] `run_tests`: auto-detects runner, maps failures via graph and SessionState
- [ ] Cold index < 3s for 10K files, warm start < 500ms via BLAKE3 cache
- [ ] Benchmark harness runs SWE-bench Pro public set and emits comparable results
- [ ] `cargo test` passes, `cargo clippy -- -W clippy::pedantic` clean, `cargo build --release` succeeds
- [ ] No `println!`, no `.unwrap()` outside tests, no `todo!` / `unimplemented!` in committed code
- [ ] README documents known limitations honestly (not a silver bullet, measured lift is +1-3pts, comparison to WarpGrep/vexp/code-graph-mcp is included)
