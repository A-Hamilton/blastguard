---
name: rust-complexity-audit
description: Audit a Rust crate for time/space complexity footguns — O(n²) scans, unnecessary clones, hot-path allocations, BTreeMap-where-HashMap-is-right, and Vec-linear-search patterns that should be hash-lookups. Produces a ranked list with file:line anchors and one-paragraph fix proposals per issue. Use proactively whenever the user mentions "slow", "performance regression", "this shouldn't be O(n²)", "why is cold index slow", or after landing a feature that touches indexing, parsing, search, or the graph types. Does NOT auto-apply fixes — surfaces the top 5 candidates for user review first.
disable-model-invocation: true
---

# Rust Complexity Audit

User-only: Claude will not invoke this automatically. Type `/rust-complexity-audit` (optionally with a path like `/rust-complexity-audit src/search/`) to run.

## When this fires

This skill is for **diagnosing and proposing** — not for auto-fixes. It's the "here's what's suspicious" pass you run before deciding to optimise.

## Procedure

1. **Scope the audit.** If the user named a path, work inside that path only. Otherwise default to `src/` (skip `src/bench/`, `tests/`, and `target/`).

2. **Run the fast signals in parallel:**
   - `cargo clippy --all-targets -- -W clippy::perf -A warnings`
   - `cargo clippy --all-targets -- -W clippy::nursery -A warnings`
   - Grep for known footguns in the scoped path:
     - `for .* in .* { .* for .* in .*` — nested-loop scans (may be O(n²))
     - `\.clone\(\)` inside function bodies that appear in hot paths (per `docs/MICROBENCH.md` + `bench/tasks_registry.py`-referenced modules)
     - `Vec::contains\(`, `\.iter\(\)\.find\(` on collections that could be hash-based
     - `BTreeMap` / `BTreeSet` where ordering isn't load-bearing — `HashMap` / `FxHashMap` is faster for BlastGuard's graph ops
     - `String::from\(.*\.to_string\(\)` double-allocations

3. **Rank the hits.** Priority order:
   - (a) hot-path allocations or scans inside `src/index/`, `src/graph/`, `src/parse/`, `src/search/` — these run per-file or per-query and compound on large repos;
   - (b) anything in `src/edit/` (apply_change is latency-sensitive);
   - (c) everything else.

4. **Produce a single report.** For each of the top 5 issues:
   - `file:line` anchor.
   - One-line observation ("double-pass grep over the full edges set on every query").
   - One-paragraph proposed fix ("replace the linear scan with the canonical-module-id lookup — same pattern as `imports_of` after commit 4e8ece1").
   - Rough complexity delta: `O(total_edges) → O(1)`, `O(files²) → O(files × avg-imports)`, etc.

5. **Do NOT apply changes.** This is a surfacing pass. The user decides which ones to pursue, and the actual implementation goes through the normal plan → implement → review loop.

## Output format

One Markdown section, `## Findings`, with the 5 ranked items. End with a single-line **Recommendation**: the one issue most worth chasing next.

## Anti-patterns this catches well

- Full-graph linear scans dressed up as `for e in graph.edges` (should usually be `forward_edges.get(&key)`).
- `.collect::<Vec<_>>()` followed by a `.iter().find()` — remove the `collect` and keep the iterator.
- `.to_string()` on a `&str` that's then borrowed — just use `&str`.
- `RefCell` in parallel code paths (use `RwLock` / `Mutex`, or better: redesign to avoid shared mutation).
- Per-call recompilation of regexes — should live in `OnceLock` like `src/search/query.rs` already does.

## Anti-patterns this does NOT catch

- Algorithmic improvements (switching a BFS to an A*, etc.) — those need design work.
- Cross-process IPC bottlenecks (MCP stdio latency is its own topic).
- Allocation patterns that only matter under specific real-world inputs (needs benchmark evidence from `bench/microbench.py`).

## After running

If the user likes one of the top-5 items, the next step is a standard brainstorm → plan → implement loop. If none of the items look worth doing, record the audit as a no-op in the session notes — "audited, nothing actionable" is a valid outcome.
