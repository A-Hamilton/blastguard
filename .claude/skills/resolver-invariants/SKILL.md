---
name: resolver-invariants
description: Invoke when edits touch src/index/indexer.rs, src/index/watcher.rs, src/edit/apply.rs, src/graph/types.rs::remove_file, or any new reindex path. Ensures the resolver trio runs after every remove_file + re-insert cycle so cross-file callers survive.
user-invocable: false
---

# Resolver Invariants

BlastGuard's graph resolver chain has a subtle invariant that is easy to break. This skill codifies the invariant so future edits don't silently re-introduce a bug that was fixed across commits `1d5e9c6` and `d9a11af`.

## The invariant

Whenever code removes a file from the graph and re-inserts its symbols/edges (cold start, warm start, file-watcher reindex, or `apply_change`'s reparse step), **all three of the following must run, in order, before any caller reads the graph**:

1. `crate::parse::resolve::resolve_imports(&mut graph, project_root)`
2. `crate::parse::resolve::resolve_calls(&mut graph)`
3. `graph.restitch_reverse_edges_for_file(&file)` — for the specific re-inserted file

Skipping step 3 is the subtle trap. `remove_file` intentionally keeps OTHER files' forward edges dangling so `detect_orphan` can still fire; that means `reverse_edges[newly_reinserted_symbol]` is empty until `restitch_reverse_edges_for_file` runs. `callers()` silently returns empty and `detect_signature` reports zero cross-file callers.

## The four call sites

Current occurrences of the trio — **keep in sync**:

| Call site | File | Purpose |
|-----------|------|---------|
| `cold_index` | `src/index/indexer.rs` | Full project scan on startup |
| `warm_start` | `src/index/indexer.rs` | Cache-assisted reindex on changed files |
| `handle_event` | `src/index/watcher.rs` | Live reindex on file save via `notify` |
| `orchestrate` step 4 | `src/edit/apply.rs` | After `apply_change` rewrites a file |

If you're adding a fifth call site, it probably also needs the trio.

## Why `remove_file` keeps dangling edges

`CodeGraph::remove_file` in `src/graph/types.rs` is deliberately asymmetric:

- It drops `forward_edges[symbol_id]` for every symbol in the removed file.
- It drops `reverse_edges[symbol_id]` for every symbol in the removed file.
- It **does not** touch other files' `forward_edges` that happen to point at the removed file's symbols. Those edges become dangling.

Why: `detect_orphan` (SPEC §5) iterates those dangling edges to surface ORPHAN cascade warnings. If `remove_file` also pruned them, the cascade detection would silently miss references to deleted symbols.

The consequence is that after re-insertion the reverse index is stale — `restitch_reverse_edges_for_file` is what repairs it.

## Regression tests that pin the invariant

If you change any of the reindex paths, at minimum run:

```bash
cargo test --test integration_apply_change signature_edit_cross_file_python_cascade
cargo test --test integration_watcher watcher_preserves_cross_file_callers_on_reindex
```

Both tests use a 2-file Python fixture where `login()` in `utils/auth.py` has exactly one cross-file caller (`handle()` in `handler.py`). If `restitch_reverse_edges_for_file` or a `resolve_*` call is missing, these fail deterministically.

## Checklist before approving a reindex-path change

- [ ] `resolve_imports` runs after re-insertion.
- [ ] `resolve_calls` runs after `resolve_imports`.
- [ ] `restitch_reverse_edges_for_file` runs for each re-inserted file.
- [ ] `signature_edit_cross_file_python_cascade` passes.
- [ ] `watcher_preserves_cross_file_callers_on_reindex` passes.
- [ ] `cargo clippy -- -W clippy::pedantic -D warnings` clean.
