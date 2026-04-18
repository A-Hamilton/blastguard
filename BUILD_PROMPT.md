# BUILD PROMPT — BlastGuard Implementation

Give this to Claude Code, Cursor, or another capable coding agent in an empty project directory along with `SPEC.md` and `CLAUDE.md` files.

---

## The prompt to paste

```
You are building BlastGuard — an open-source Rust MCP server for AI coding agents. Two files are attached as the authoritative source:
- SPEC.md — complete technical specification
- CLAUDE.md — build order, phases, and quality gates

Goal: maximize SWE-bench Pro benchmark performance while keeping token efficiency and speed as first-class constraints. The honest projected lift is +1 to +3 points on SWE-bench Pro with wide variance. The only way to know if it works is to build it, benchmark it rigorously, and publish the results.

## Before writing code — research phase

Do not start implementation until you have verified the following. Use web search and read the cited papers where practical.

1. Check the current SWE-bench Pro leaderboard. Confirm Opus 4.7's score (expected: 64.3%) and GLM-5.1's score (expected: 58.4%). Note any newer models released.

2. Verify the latest versions of the dependencies in SPEC.md §12 on crates.io. Specifically: rmcp, tree-sitter + grammars, notify, sqlite-vec, fastembed. If any have major version bumps, update the spec.

3. Look up the peer-reviewed lift data referenced in CLAUDE.md to ground your expectations:
   - cAST paper: +2.67 Pass@1 on SWE-bench
   - WarpGrep v2: +2.1-2.2 on SWE-bench Pro
   - Auggie semantic retrieval: +6 over SEAL baseline
   - CodeCompass AST graph MCP: +20pp on hidden-dependency tasks, 0pp on semantic tasks

4. Check if code-graph-mcp or vexp have published newer benchmark results. If they have beaten BlastGuard's projected range, reconsider scope.

5. Read https://arxiv.org/abs/2509.16941 (SWE-bench Pro paper) §4.4 for failure mode analysis. Your implementation should directly address context overflow (35% of failures) and multi-file cascade errors.

Report back with what you found before writing any code.

## Implementation approach

Follow SPEC.md exactly. The phases in CLAUDE.md are not optional — they reflect evidence-backed scope discipline.

**Phase 1 — ship minimum viable product:**
- Three tools: search (graph queries + grep), apply_change (with 4 cascade warnings), run_tests (with failure-to-code mapping via the graph)
- Four languages: TypeScript, JavaScript, Python, Rust
- BLAKE3 Merkle tree cache for sub-500ms warm starts
- Benchmark harness included in MVP deliverables

**Phase 2 — add only if Phase 1 benchmark data supports effort:**
- Semantic search via sqlite-vec + fastembed (cargo feature flag)
- `around X` bundled retrieval pattern
- Additional cascade checks based on instrumentation

Do not skip the benchmark harness. Without measurement, the design is untested.

## Rust quality gates

Run after every phase:
- `cargo build --release` must succeed
- `cargo clippy -- -W clippy::pedantic` must produce zero warnings
- `cargo test` must pass

Code standards:
- No .unwrap() in production paths
- /// doc comments on every public item
- #[must_use] on Result-returning functions
- Per-module #[cfg(test)] mod tests
- tracing::info! / tracing::error! to stderr — no println!
- Prefer &str over String where possible

## Token efficiency principles

Every tool response must be measured against a token budget. Targets from SPEC.md §3:
- Structural search results: 50-300 tokens
- Grep results: 100-400 tokens (cap 30 matches)
- apply_change with warnings: 100-300 tokens
- apply_change clean: 40-80 tokens
- run_tests pass: 30-50 tokens
- run_tests with failures: 80-200 tokens

Measure actual token usage during the benchmark runs. If responses exceed budget, compress — do not dump raw AST.

## Speed principles

Cold index of a 10K-file project: under 3 seconds. Warm start: under 500ms. Achieve through:
- BLAKE3 Merkle tree to skip unchanged directory subtrees
- rayon parallel parsing, one tree-sitter parser per thread
- rmp-serde cache format (compact and fast)
- ignore crate (ripgrep's walker, respects .gitignore)

Single file reindex under 50ms. Impact analysis under 10ms. Search under 100ms.

If any of these targets is missed, profile and fix before proceeding to the next phase.

## Benchmark-driven development

The benchmark harness (SPEC.md §15) is not a stretch goal. It is the measurement infrastructure that makes every design decision testable.

Required:
- Runs SWE-bench Pro public set (731 tasks)
- Baseline: mini-SWE-agent v2 or equivalent scaffold without BlastGuard
- Test: same scaffold with BlastGuard MCP enabled
- Compare with and without per model: Opus 4.7 (expensive) and GLM-5.1 (cheap sanity check)
- Instrument every tool call: input tokens, output tokens, wall time, cache hits
- Report: resolution rate delta, token delta, turn count delta, per-repository breakdown

After Phase 1 MVP is complete, run the benchmark. Commit results to the repository as honest evidence. If results are negative or flat, that is valuable data.

Only add Phase 2 features if Phase 1 data supports the additional complexity.

## Agent behavior when using BlastGuard

The tools do not force usage. Per research (CodeCompass paper), forced tool use is unproven. Agents must choose to use BlastGuard based on task complexity. For this to work, tool descriptions must be honest and specific about when each tool is useful. See SPEC.md §3 for the exact descriptions.

The target CLAUDE.md snippet that ships in BlastGuard's README (for users to add to their own projects) should read approximately:
"For multi-file changes where seeing blast radius matters, use BlastGuard's apply_change. For trivial single-line fixes, your native edit tool is fine. Use search's 'around X' pattern when exploring an unfamiliar function."

## Honest positioning

The README.md must include:
- Measured benchmark lift with confidence intervals (not projected numbers)
- Comparison table against vexp (commercial), code-graph-mcp (open source), WarpGrep (closed)
- Known limitations: dynamic dispatch blind spots in Python and JavaScript, no Go support in Phase 1, graph stale between file watcher debounce windows
- Acknowledgment that Opus 4.7 and GLM-5.1 already handle much of what BlastGuard provides natively — BlastGuard's value is strongest on weaker/cheaper models

Do not over-promise. Benchmark hacking is rampant in this space (see Berkeley's recent paper on benchmark exploits). Integrity is a differentiator.

## Deliverables

When done, the repository should contain:
1. Source code matching SPEC.md structure (§14)
2. Complete test suite passing
3. Benchmark harness and instructions to reproduce
4. Published benchmark results in README.md with raw trajectories in bench/results/
5. Comparison table against competitors
6. Honest limitations section
7. A CHANGELOG.md
8. An MIT LICENSE

Ready to start? Begin with the research phase. Report back with:
- Current SWE-bench Pro scores (Opus 4.7, GLM-5.1, top 5 overall)
- Dependency version check results
- Any recent research or competitor releases that affect scope
- Your proposed adjustments to SPEC.md based on findings, if any

Then proceed to Phase 1 implementation. Ship the MVP. Benchmark. Iterate based on evidence.
```

---

## Supplementary notes for human before handing off

- Give the AI permission to write files aggressively. It will create ~20-30 files.
- Budget 4-8 hours of agent time for Phase 1 MVP, excluding benchmark runs.
- Benchmark runs themselves take 4-24 hours depending on parallelism and model choice.
- Opus 4.7 benchmark cost estimate: ~$50-150 for the 731 public set tasks with default scaffolding.
- GLM-5.1 benchmark cost estimate: ~$10-25 for the same set.
- If results are flat or negative, that is publishable and useful. Do not pressure the agent to report success.
