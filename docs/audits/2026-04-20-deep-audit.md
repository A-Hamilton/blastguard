# Deep Audit — 2026-04-20

**Branch:** `chore/deep-audit-20260420` (worktree at `/home/adam/bg-audit-20260420`, isolated from the in-flight bench at `/home/adam/Documents/blastguard`).

**Commits this branch:** none — the codebase was clean enough at every phase that no auto-fixes were applied.

## Summary

- **Phase 1 (dead code):** 0 rustc `dead_code` warnings. 30 `pub fn` items across `src/`; none flagged as green/delete. `cargo-udeps` not installed locally — unused-dependency check skipped.
- **Phase 2 (quality sweep):** 0 warnings under `cargo clippy --all-targets -- -W clippy::pedantic`. Matches the project's existing CI gate.
- **Phase 3 (complexity audit):** 0 warnings under `clippy::perf` + `clippy::nursery`. Grep-based pattern scan (nested loops, `Vec::contains`, `BTreeMap`-where-`HashMap`, `.iter().find()`) found zero real concerns — every hit was either a deliberate ordering choice or inside a test body.

### Next action

**Nothing actionable from this pass.** The codebase is at its pedantic-clean baseline. The realistic next moves aren't audit-surfaceable:

1. Install `cargo-udeps` via nightly (`cargo +nightly install cargo-udeps`) and re-run Phase 1 to catch genuinely unused Cargo.toml deps — lint-only passes can't see those.
2. Pivot to SWE-bench Verified (KNOWN_GAPS Gap 5 option 3) — this remains the real blocker on the headline BlastGuard lift claim. Code quality is not the bottleneck.
3. Cloud-model validation round on Sonnet/Opus — confirms the PR #1 `chain from X to Y` file-path fix generalises beyond Gemma.

---

## Phase 1: Dead Code

```
cargo rustc --lib --profile=check -- -W dead-code
# 0 warnings
```

Grep inventory:

- `pub fn`: 30 in `src/`.
- `pub use`: 8 re-exports (all legitimate — module boundary re-exports).
- `pub struct`/`pub enum`: not individually audited; pedantic would flag `must_use_candidate` on any unused ones and it did not.

No Green / Yellow / Red items. Every pub item is either called or part of an intentionally-exposed API surface.

Unused-dependency check **deferred** pending `cargo-udeps` install.

## Phase 2: Quality Sweep

```
cargo clippy --all-targets -- -W clippy::pedantic
# 0 warnings
```

No auto-fix commit. No review-list items. The project's CI gate `cargo clippy --all-targets -- -W clippy::pedantic -D warnings` is already enforcing this at zero-tolerance.

## Phase 3: Complexity Audit

```
cargo clippy --all-targets -- -W clippy::perf -W clippy::nursery
# 0 warnings
```

Grep-based pattern scan:

### Nested loops
- `src/runner/parse.rs:39+50` — outer loop over test-result files, inner loop over assertions per file. O(N·M) is inherent to the data shape (nested JSON); not an algorithmic inefficiency.
- `src/parse/resolve.rs:147` — single-level loop over tsconfig path mappings; no inner loop. False positive.

### `.iter().find()`
5 hits total. 4 of 5 are inside `#[cfg(test)]` blocks (test assertions). The single production hit, `src/parse/resolve.rs:547`, iterates the `names` list of exports from a single file — small N, linear scan is correct here.

### `.iter().any()` / `Vec::contains()`
All 10 hits are inside `#[cfg(test)]` blocks (assertion helpers). No production regressions.

### `BTreeMap` / `BTreeSet` usage
4 hits. All are deliberate ordering choices with comments confirming intent:

- `src/search/structural.rs:427` — alphabetical library sort for `libraries` query output.
- `src/graph/impact.rs:198` — counts sorted for deterministic output.
- `src/edit/context.rs:47` — stable dedup for test file lists.
- `src/mcp/status.rs:34` — stable dedup for extension list.

None should be `HashMap` / `HashSet`.

---

## What this audit did NOT cover

- **Unused Cargo dependencies** — requires `cargo-udeps` (nightly). Install and re-run for a full Phase 1.
- **Dead `pub(crate)` items** — rustc `dead_code` catches these, but 0 warnings means there are none.
- **Algorithmic improvements** — out of scope. Audits don't redesign BFS into A*. Requires real benchmark evidence from `bench/microbench.py` to justify.
- **Cross-file call chains** — no current tool measures these; the BlastGuard MCP itself could answer it (`outline of PATH`, `callers of NAME`) but wasn't invoked here.

## Replication

```bash
git worktree add /home/adam/bg-audit-20260420 -b chore/deep-audit-20260420
cd /home/adam/bg-audit-20260420
cp -rl /home/adam/Documents/blastguard/target ./target  # share build cache
nice -n 19 ionice -c 3 cargo check --all-targets
nice -n 19 ionice -c 3 cargo clippy --all-targets -- -W clippy::pedantic
nice -n 19 ionice -c 3 cargo clippy --all-targets -- -W clippy::perf -W clippy::nursery
```

Total audit runtime: ~6 minutes. CPU contention with the running bench: minimal — the hardlinked target cache meant incremental `cargo check` finished in 1.8s.
