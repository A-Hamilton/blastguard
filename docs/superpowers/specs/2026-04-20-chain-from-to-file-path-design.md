# `chain from X to Y` — file-path endpoint

**Status:** Design, approved to proceed to plan.
**Date:** 2026-04-20.
**Scope:** Tool-level enhancement to BlastGuard's `chain from X to Y` search query so it can close the chain-search-to-graph regression observed across bench rounds 9–11.

## Problem

The bench task `chain-search-to-graph` asks the agent to name the function chain from the MCP `search` tool entry point down into the code-graph module. The answer is `src/mcp/server.rs::search_tool → src/search/dispatcher.rs::dispatch → <some function in src/search/structural.rs>`.

Today's `chain from X to Y` query requires **both** endpoints to be named symbols. But the agent on this task knows the target **module** (`structural.rs`) and not a specific function inside it — `dispatch` routes to ~10 different structural callees via a `match QueryKind` arm. The agent has no sensible value to put as `Y`, so either falls back to multi-step bash/grep work or prematurely emits DONE after two hops.

Three bench rounds iterating the prompt failed to fix this — Gemma-4 produces three different failure modes for the same underlying tool shortfall. See `docs/MICROBENCH.md` rounds 9–11 for evidence.

## Goal

Close the regression at the tool layer: enable `chain from X to FILE` so a single BlastGuard query returns the full call chain plus all candidate endpoints in the target file. Success = BG passes the Priority 1a substring grader on `chain-search-to-graph` without regressing `outline-tree-sitter-rust` or `find-tamper-patterns`.

## Non-goals

- Re-export traversal through `pub use` chains (option C in brainstorm). Deferred — not the failure cause for this task.
- A separate `callees of X in FILE` query (option B). Narrower than needed; the task is multi-hop.
- Any change to the prompt beyond one line of cheat-sheet update. Further prompt iteration is expressly out of scope per bench round-11 write-up.

## Approach — option (A) from brainstorm

Extend `chain_from_to` to detect when `Y` is a file path and run a path-terminated BFS instead of the current name-to-name BFS.

### Changes

**1. `src/graph/ops.rs`** — add a predicate-terminated BFS helper.

```rust
#[must_use]
pub fn shortest_path_to_predicate<F>(graph: &CodeGraph, from: &SymbolId, pred: F) -> Option<Vec<SymbolId>>
where F: Fn(&SymbolId) -> bool { ... }
```

The existing `shortest_path(graph, from, to)` is refactored to a one-line wrapper: `shortest_path_to_predicate(graph, from, |id| id == to)`. Pure structural refactor; no behavior change for the existing call sites.

**2. `src/search/structural.rs::chain_from_to`** — add a path-mode branch.

- **Detection** — `to_name` is treated as a path when it contains `/` or `\` or when its trailing segment ends in one of `{.rs, .ts, .tsx, .js, .jsx, .py}`. Bare identifiers and qualified symbol names (e.g. `module::fn`) never trip this. Conservative on purpose; ambiguity defaults to the existing symbol-name behavior.
- **Target normalization** — the target path is converted to a `PathBuf` and compared against each node's `SymbolId.file` using `Path::ends_with`, which compares **path components** (not bytes). `SymbolId.file` is stored absolute today (see the `strip_prefix(project_root)` call sites in `src/search/structural.rs`, `src/edit/apply.rs`, etc.), so `/home/.../blastguard/src/search/structural.rs` matches the agent's `src/search/structural.rs` cleanly via component-wise `ends_with`. Bare filenames (`structural.rs`) also match, at the cost of ambiguity when two files share a basename — acceptable tradeoff given the agent can always qualify further.
- **BFS** — `shortest_path_to_predicate(graph, from_id, |id| id.file.ends_with(&target))` returns the shortest chain to the first node inside the target file.
- **Result augmentation** — after the chain, append one structural `SearchHit` for every **other** symbol in the target file that is a direct `Calls`-edge successor of any node on the returned chain. This gives the agent the full candidate-endpoint set for "which specific function in `structural.rs` does `dispatch` hit?" in a single response.
- **Ordering in the response** — chain nodes first, in path order; candidate endpoints after, ordered by centrality descending (so the most-called structural helper sits near the top).

- **Fallback paths:**
  - File has zero indexed symbols → single `SearchHit::empty_hint("no symbols indexed in {path}; try `outline of {path}`")`.
  - File indexed but no call-graph path reaches it → `from_id` as a structural hit plus an `empty_hint("no call-graph path from {from} to any symbol in {path}; try `callees of {from}` then filter by file")`. Mirrors the existing name-mode fallback shape so downstream consumers don't need a new branch.
  - `from_name` doesn't resolve → same `empty_hint` as today, unchanged.

**3. `bench/prompts.py::BLASTGUARD_BIAS`** — one-line update under the `chain from A to B` bullet to note that `B` may be a file path, with the canonical example:

```text
`blastguard_search '{"query":"chain from search_tool to src/search/structural.rs"}'`
```

No other prompt edits. The rest of the cheat-sheet stays as it landed in round 11.

**4. `src/search/query.rs`** — no code change. The regex already captures `Y` verbatim as a `String`. Path discrimination is a `structural.rs` concern.

### Data flow

```
agent → blastguard_search "chain from search_tool to src/search/structural.rs"
  → query::classify → QueryKind::Chain("search_tool", "src/search/structural.rs")
  → dispatcher::dispatch → structural::chain_from_to
    → is_path_like("src/search/structural.rs") = true
    → shortest_path_to_predicate(g, from, |id| id.file.ends_with("src/search/structural.rs"))
    → returns chain: [search_tool, dispatch, <first structural hit>]
    → augment with other Calls-successors of dispatch in structural.rs
  → Vec<SearchHit>
```

### Behavioral contract

- Symbol-mode (`Y` is a bare name): identical to today.
- Path-mode (`Y` is path-like): new. Returns `[chain...] ++ [file_candidates...]` or the documented empty-hint fallbacks.
- The wire shape stays `Vec<SearchHit>`. No change to `BundledContext`, `SearchHit`, or the MCP tool JSON schema.

## Testing

Unit tests colocated in each touched file:

- `src/graph/ops.rs`:
  - `shortest_path_to_predicate_walks_forward_edges` — single-match predicate reaches the target.
  - `shortest_path_to_predicate_none_when_no_node_matches` — predicate returns false everywhere.

- `src/search/structural.rs`:
  - `chain_to_file_path_returns_chain_plus_file_candidates` — two-hop chain whose terminal hop lands in target file; asserts the chain order and the augmented candidate list (with centrality-ordered extra hits).
  - `chain_to_file_path_backward_compat_with_symbol_name` — bare symbol name still routes through the existing shortest_path-by-name branch. Asserts the existing `chain_from_to_returns_shortest_path` behavior is unchanged.
  - `chain_to_file_path_unreachable_falls_back_with_hint` — file is indexed but no forward path reaches it; asserts `from` hit + empty-hint.
  - `chain_to_file_path_when_from_already_in_target_file` — `from_id.file == target`; chain is `[from]`, candidates are its direct same-file Calls-successors.
  - `chain_to_file_path_when_file_not_indexed` — unknown path; asserts the "no symbols indexed" empty-hint.

Full `cargo test` must pass with zero new warnings under `cargo clippy -- -W clippy::pedantic -D warnings`.

## Verification before "done"

1. `cargo check --all-targets`
2. `cargo test`
3. `cargo clippy --all-targets -- -W clippy::pedantic -D warnings`
4. `cargo build --release`
5. Bench re-run on Gemma 4 with the round-11 task set:
   ```bash
   bench/.venv/bin/python -m bench.microbench \
     --api-base http://127.0.0.1:8080/v1 --api-key-env DUMMY_KEY \
     --model gemma-4 --model-id-override gemma-4 \
     --tasks chain-search-to-graph outline-tree-sitter-rust find-tamper-patterns \
     --seeds 1 --run-judge --judge-n 3 \
     --output bench/runs/$(date +%Y%m%d-%H%M%S)-round12-chain-to-file.jsonl
   ```
6. Acceptance signals:
   - Priority 1a grader reports `chain-search-to-graph / blastguard: correct=True` with `structural` in missing=[]. This is the primary claim being made.
   - `outline-tree-sitter-rust` and `find-tamper-patterns` BG grader outcomes unchanged from round 11 (no new regressions).
   - BG input-token delta on chain-search-to-graph does not exceed round-11's +62% baseline by more than 10pp (Priority 2 gate).
7. Append a Round 12 section to `docs/MICROBENCH.md` with the honest outcome, whatever it is.

## Risks & mitigations

- **BFS terminates too early on a shallow file match.** E.g. a logging call in `src/mcp/foo.rs` happens to share a path segment with the target. Mitigated by `ends_with` path comparison on the canonical project-relative file path, not substring matching, and by the agent writing a specific enough target path to disambiguate.
- **Backward compatibility surprise.** A symbol literally named `a.rs` (unlikely but legal in some languages) would now be interpreted as a path. Mitigated by the "ends in source extension" rule being necessary, not sufficient — `a.rs` on its own would match, but this is an extreme edge case and the worst outcome is an empty chain which matches today's behavior.
- **Candidate-endpoint blow-up on large files.** Cap the candidate list at 10 (reusing the existing per-query cap constant from `edit/context.rs` where one exists; introduce a local constant if not). Above 10, emit a hint noting more exist.
- **Prompt cheat-sheet drift.** Update is a single bullet; no other edits this round.

## Out of scope (explicit deferrals)

- Re-export edge traversal.
- `callees of X in FILE` as a separate query.
- Multi-endpoint chain search (`chain from X to any of [A, B, C]`).
- Any Gemma-specific prompt retuning.

## Rollback

Revert the four-file diff. The change is purely additive behind the path-like detection guard; symbol-mode callers see no change.
