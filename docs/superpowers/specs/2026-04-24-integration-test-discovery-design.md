# Integration-test discovery via crate-aware import resolution

**Status:** Design, not yet approved — follow-up spec for the third gap identified in Round 13 (after `callers-of-with-context` shipped in PR #20).
**Date:** 2026-04-24.
**Scope:** Fix `tests for NAME` so it finds integration tests in `tests/*.rs` that import the library via the crate's external name (e.g. `use blastguard::edit::apply_change`).

## Problem

Round 13 `tests-for-apply-change` showed BG losing to raw 2/3 on the judge substance axis. Raw found integration tests in `tests/integration_apply_change.rs` and `tests/integration_mcp_server.rs`; BG missed them.

**Root cause (diagnosed 2026-04-24 via live MCP probe):**

- BlastGuard's `tests_for` query does: find target symbol → lookup `importers_of(target.file)` → filter by `is_test_path`. This is correct for **intra-crate** tests (e.g. `#[cfg(test)] mod tests` inside the same file, or `tests/helpers.rs` files that `use crate::...`).
- In Rust, integration tests at `tests/*.rs` are compiled as **separate crates** that link the library as a dependency. They import via the external crate name: `use blastguard::edit::apply_change`.
- The Rust parser (`src/parse/rust.rs`) treats `use <external_name>::...` as an external library import and emits a `library_imports` entry, NOT an intra-crate `Imports` edge. So `importers_of(src/edit/mod.rs)` misses every integration test.

**Live verification of the bug:**

```bash
$ blastguard_search '{"query":"importers of src/edit/mod.rs"}'
# returns only src/mcp/apply_change.rs — missing all integration tests

$ blastguard_search '{"query":"tests for apply_change"}'
# returns "no same-file tests found" — false negative
```

## Goal

`tests for NAME` should return intra-crate tests **plus** integration tests that import the library containing NAME. Success = on the BlastGuard repo, `tests for apply_change` returns both the intra-file tests in `src/edit/mod.rs` AND the integration tests in `tests/integration_apply_change.rs`.

## Non-goals

- Multi-crate workspace handling (single-crate repos only for Phase 1; workspaces are separate work).
- Python's or TS/JS's equivalent patterns — scope this to Rust first. Python uses `sys.path` sibling imports which are already intra-graph. TS/JS monorepo imports are workspace-level.

## Approach: crate-aware import resolution (root-cause fix)

### Component 1: read crate name at index time

`src/index/indexer.rs` (or wherever project metadata is discovered) reads `Cargo.toml` at the project root once and extracts `[package].name`. Store it as `project_crate_name: Option<String>` on the index context.

For workspaces: read each workspace member's name into a set. Out of scope for Phase 1 but design allows a `HashSet<String>` instead of `Option<String>` for future extension.

### Component 2: rewrite external-crate imports to intra-crate

In `src/parse/rust.rs::emit_use`, when processing a `use_declaration`:

1. If the first path segment matches `project_crate_name`, treat the rest as an intra-crate `crate::<rest>::` path.
2. Resolve that path through the existing `resolve_imports` pipeline, same way `use crate::edit::apply_change` is resolved today.
3. Emit an intra-crate `Imports` edge pointing at the resolved target file, NOT a `library_imports` entry.

No change to the tree-sitter query — the AST node is `use_declaration` either way. Only the post-parse handling branches.

### Component 3: pass crate name through the parse pipeline

`parse::rust::extract` gains a `crate_context: Option<&str>` parameter. The indexer passes it based on the loaded `Cargo.toml` info. Tests may pass `None` for isolated unit tests.

## What this unlocks

With Component 2 landed, the existing `tests_for` query works as-is. No changes to `structural::tests_for`, `query.rs`, or the dispatcher. The fix is entirely at the indexer layer.

After landing, live probe should show:

```bash
$ blastguard_search '{"query":"importers of src/edit/mod.rs"}'
# returns src/mcp/apply_change.rs PLUS tests/integration_apply_change.rs + tests/integration_mcp_server.rs
```

## Testing

### Unit tests (new)

- `parse::rust::extract_rewrites_self_crate_use` — `use blastguard::foo::bar` with `crate_context=Some("blastguard")` produces the same `Imports` edge as `use crate::foo::bar`.
- `parse::rust::extract_leaves_external_use_alone` — `use tokio::spawn` stays in `library_imports`.
- `parse::rust::extract_handles_none_crate_context` — no rewriting when context isn't provided.

### Integration tests

- `tests_for_apply_change_finds_integration_tests` (at `tests/` level) — index this repo, query `tests for apply_change`, assert the result includes both `tests/integration_apply_change.rs` and at least one intra-file test.

### Live verification

- Against the real BlastGuard repo: after the feature lands, `tests for apply_change` should return ≥ 3 hits including at least one `tests/integration_*.rs`.

## Risks & mitigations

- **False positives on self-name shadowing.** If a user writes `use blastguard` where `blastguard` is actually a local module (not the crate), we'd mis-resolve. Mitigated by: `crate_context` is only set from `Cargo.toml`'s `[package].name`, so we're matching the real crate name. Collisions are rare and can be tested against.
- **Reindex cost.** None — the detection is a one-time `Cargo.toml` read at index start.
- **Workspaces.** Out of scope; Phase 1 single-crate only. The `HashSet<String>` shape allows future expansion without API change.

## Rollback

Revert the `src/parse/rust.rs` change and the `crate_context` plumbing. No data-format changes; no index-cache invalidation needed.

## Out of scope (explicit deferrals)

- Python / TS / JS equivalents (different import models).
- Multi-crate workspaces.
- Following `pub use` re-export chains from integration tests (harder; should still give a correct one-step hit which is enough for `tests_for`).
