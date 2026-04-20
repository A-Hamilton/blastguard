---
name: dead-code-sweep
description: Find unused Rust symbols (functions, structs, enums, modules, `pub use` re-exports) and unused Cargo dependencies in the BlastGuard crate. Produces a ranked deletion candidate list with file:line anchors. Use proactively when the user says "is this still used", "find dead code", "what can I delete", "unused dependencies", "slim down", or after a refactor that might have orphaned helpers. The goal is evidence for a cleanup PR — not to auto-delete. Tends to find: pre-Phase-2 helper stubs, experimental parsers, fields on rarely-hit error types, and deps that moved from `[dependencies]` to `[dev-dependencies]`-appropriate.
disable-model-invocation: true
---

# Dead Code Sweep

User-only: Claude will not invoke this automatically. Type `/dead-code-sweep` (or `/dead-code-sweep src/parse/`) to run.

## Why a skill vs just `cargo +nightly rustc -W dead-code`?

Rustc's `dead_code` lint is a good start but has two limits on this repo:

1. It doesn't follow `pub` visibility — everything marked `pub` counts as used even if no external crate imports it. Since BlastGuard is a binary crate with `pub(crate)` encouraged by `CLAUDE.md`, there are many `pub` items that are genuinely unused inside the crate.
2. It doesn't report unused dependencies — those need `cargo-udeps` (nightly) or `cargo-shear`.

This skill combines both sources, filters out false positives (benchmark-only helpers, test-only fixtures), and ranks by deletion confidence.

## Procedure

1. **Verify tools.** If `cargo +nightly udeps` is installed, use it. Otherwise, skip the unused-deps section and note it. If nightly isn't installed at all, use stable `cargo rustc -- -W dead-code` instead of nightly.

2. **Dead code scan (rust items):**
   - `cargo rustc --all-targets --profile=check -- -W dead-code 2>&1 | grep -E "function is never used|struct is never constructed|never read"`
   - For each hit in the scoped path:
     - Grep the crate for string references to the symbol — sometimes a `pub` item is used via a trait method call that the lint can't follow.
     - Check `git log --oneline -S <symbol> -- src/` — when was it added? Why? (a recent addition might be part of an unfinished Phase-2 feature; an old one is a safer delete)

3. **Unused dependencies scan:**
   - `cargo +nightly udeps --all-targets 2>&1 | tail -40` (if available)
   - Cross-reference against `Cargo.toml` comments — BlastGuard's `Cargo.toml` sometimes has explanatory comments on why a dep is present; respect those.

4. **Unused re-exports scan (`pub use X::Y`):**
   - Grep for `pub use` lines.
   - For each, check if anything outside the module references `Y` via the re-export path. If not, it's a candidate for either removal or downgrading to `pub(crate) use`.

5. **Rank by deletion confidence.**
   - **Green (high confidence):** `pub(crate) fn` / `fn` items with zero references in the crate. Delete these with a one-commit PR.
   - **Yellow (medium):** `pub` items with zero external callers. Consider downgrading to `pub(crate)` first, then a follow-up removal if nothing complains.
   - **Red (needs judgment):** items whose git log says "will be wired in Phase 2" or similar. Leave with a note in KNOWN_GAPS.md if not already captured.

6. **Skip these categories.**
   - `#[cfg(test)]` blocks — their helpers are allowed to be loosely referenced.
   - `bench/` Python — out of this skill's scope.
   - `src/main.rs` — entry point, can be non-obviously referenced by Cargo.

## Output

A single Markdown report:

```
## Dead code (Rust items)

### Green — delete with confidence
- `src/foo.rs:123 fn obsolete_helper` — added 2025-11-04 in commit abcd123, zero refs today.
- ...

### Yellow — consider downgrading to pub(crate) first
- `src/graph/types.rs:88 pub fn stale_getter` — no external users; narrow visibility then recheck.
- ...

### Red — check intent before removing
- `src/phase2/scaffold.rs` — commit 9876543 says "Phase 2 scaffold, wire later".

## Dead dependencies (Cargo.toml)

- `regex-syntax` — no references in src/; removable. (Added in commit ef5432, not used today.)

## Unused re-exports

- `src/lib.rs:14 pub use foo::Bar` — no external users.

## Recommendation

Delete the Green items in one commit (~N lines, safe). Defer Yellow and Red.
```

## After running

The user picks which Green items to act on. Green-item deletion is a single-commit PR — mechanical, low-risk, easy to revert.

Yellow items should be handled one at a time: downgrade to `pub(crate)`, rerun `cargo check`, and if nothing breaks, the next sweep will promote them to Green.

Red items are design-work, not cleanup — they need a brainstorm, not a delete.
