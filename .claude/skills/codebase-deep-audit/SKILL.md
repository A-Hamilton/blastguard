---
name: codebase-deep-audit
description: One-shot deep optimisation pass across the BlastGuard Rust crate — orchestrates dead-code-sweep, rust-quality-sweep, and rust-complexity-audit in the correct order on a single umbrella branch, then writes one consolidated report. Use whenever the user asks for a "full audit", "deep pass", "optimization pass", "clean up the whole thing", "spring cleaning", or any phrase suggesting they want the codebase tightened end-to-end rather than a narrow fix. Does NOT auto-delete dead code or auto-fix judgment-call lints — the mechanical-safe changes land automatically, everything else is surfaced in the report for the user to act on.
disable-model-invocation: true
---

# Codebase Deep Audit

User-only: type `/codebase-deep-audit` (optionally with a path scope like `/codebase-deep-audit src/search/`) to run. Orchestrates the three single-purpose audit skills.

## Why run them together (and in THIS order)?

Three narrow skills exist already:

1. `dead-code-sweep` — finds unused items (functions, structs, deps, `pub use` re-exports).
2. `rust-quality-sweep` — auto-fixes the safe clippy pedantic lints and surfaces the judgment-call ones.
3. `rust-complexity-audit` — finds O(n²) scans, hot-path clones, hash-vs-tree choices.

Running them ad-hoc sometimes wastes work: complexity-audit surfaces a hot-path clone, but the function is dead code and should just be deleted. Quality-sweep adds `#[must_use]` to a function that's about to be removed. The orchestrator puts them in the right order to avoid that waste.

**Order (deliberate):**

1. **Dead-code sweep first.** Don't optimise or clean code that's about to be deleted. If the sweep flags items as Green (safe delete), the user decides whether to delete them NOW or defer; either way the downstream audits run against the post-decision code.
2. **Quality sweep second.** Auto-fixes stylistic lints on the code that's known-live. Single commit, reversible.
3. **Complexity audit last.** Runs against the cleaned, live code. Any perf concerns it surfaces are now about code that's worth optimising.

## Preconditions (hard stop if any fail)

- Working tree clean — refuse if `git status --porcelain` is non-empty. Ask the user to commit/stash first.
- Not on `main`. Create an umbrella branch `chore/deep-audit-<yyyymmdd>` if on main.
- `cargo check --all-targets` passes before we start.
- **Bench not running.** Check `pgrep -af bench.microbench` — if anything is running, refuse and tell the user. Clippy/cargo activity would steal CPU from llama-server during a benchmark.

## Procedure

1. Enforce preconditions. Abort on any failure with a clear message.

2. Create the umbrella branch if not already there: `git checkout -b chore/deep-audit-<yyyymmdd>`.

3. **Phase 1 — Dead-code sweep.**
   - Read and execute `.claude/skills/dead-code-sweep/SKILL.md` (the procedure section, not the yaml metadata).
   - Save the full report to `docs/audits/<yyyymmdd>-deep-audit.md` under a `## Phase 1: Dead Code` heading.
   - Do NOT delete anything — let the user decide. Record Green / Yellow / Red tallies in the report.
   - If the user told us (via the trigger prompt) to "be aggressive" or "delete safe stuff", delete only the Green tier in one commit named `chore: remove dead code (deep-audit N green)`. Otherwise skip.

4. **Phase 2 — Quality sweep.**
   - Read and execute `.claude/skills/rust-quality-sweep/SKILL.md`.
   - That skill already commits its own auto-fixes on the current branch — we're already on the umbrella branch, so that's fine.
   - Append the skill's "Auto-fixed" + "Review list" output to the report under a `## Phase 2: Quality Sweep` heading.

5. **Phase 3 — Complexity audit.**
   - Read and execute `.claude/skills/rust-complexity-audit/SKILL.md`.
   - Append its top-5 ranked findings to the report under `## Phase 3: Complexity Audit`.
   - This phase writes NO commits — it's a surfacing pass.

6. **Consolidated summary.** At the top of the report, write:
   - Branch name.
   - Commit SHAs from Phase 1 (if delete happened) and Phase 2 (always).
   - Count of Green dead-code items left un-deleted (so the user can come back to them).
   - Count of judgment-call quality lints left un-fixed.
   - Top-3 of the 5 complexity findings (by confidence / impact).
   - A single **Next action** recommendation — the one thing most worth the user's next 30 minutes.

7. Run `cargo check --all-targets && cargo test && cargo clippy --all-targets -- -W clippy::pedantic -D warnings` one more time to verify the auto-fixes from Phase 2 didn't break anything. Abort with a `git reset --hard` if any gate fails.

8. Print the report path and branch name to the terminal. Do NOT push — the user pushes when they're happy.

## Output shape

Report file: `docs/audits/<yyyymmdd>-deep-audit.md`

```markdown
# Deep Audit — 2026-04-20

**Branch:** chore/deep-audit-20260420
**Commits:** abc123 (Phase 1 green deletes), def456 (Phase 2 auto-fixes)

## Summary

- Dead code: 3 Green items deleted, 2 Yellow held for review, 1 Red flagged for intent check.
- Quality sweep: 14 auto-fixes across 7 files; 3 judgment-call lints held for review.
- Complexity audit: top 3 findings below.

### Next action

> Investigate `src/parse/rust.rs::extract_visibility` — the Yellow dead-code flag plus the "138 lines" too_many_lines pedantic lint suggests this function has grown unwieldy and may be ready for a split or a pivot. Worth 30 minutes before shipping anything new.

## Phase 1: Dead Code
...

## Phase 2: Quality Sweep
...

## Phase 3: Complexity Audit
...
```

## Failure modes to handle gracefully

- **Phase 2's quality-sweep breaks the tree.** The inner skill already guards against this with `git reset --hard` on test failure; the orchestrator just confirms the tree is still clean before Phase 3 starts.
- **User Ctrl-C's mid-phase.** The umbrella branch is isolated, so nothing on `main` is affected. Next run can re-branch cleanly.
- **One of the three inner skills is missing.** Abort with a clear message naming the missing skill file.

## When NOT to use this

- You've just landed a narrow feature and only want to audit that feature — run the single relevant skill instead.
- You're mid-bench or mid-experiment — wait until CPU is free.
- The codebase is in a broken state — fix it first. This skill assumes `cargo check` passes.
- You want to actually DELETE dead code — this skill defaults to surfacing. Pass an explicit instruction in the trigger prompt if you want Phase 1 to also auto-delete Green items.

## After running

Read the report. Commit nothing else to this branch unless it's a follow-up action from the report. When happy, push and open a PR titled `chore: deep audit YYYY-MM-DD` with the report as the PR body.
