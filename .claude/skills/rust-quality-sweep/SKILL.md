---
name: rust-quality-sweep
description: Run a conservative code-quality pass on the BlastGuard Rust crate — auto-fixes the **safe** clippy pedantic lints (needless_borrow, redundant_clone, must_use_candidate, items_after_statements, single_char_pattern), adds missing rustdoc on public items, and surfaces the unsafe-to-auto-apply lints for manual review. Produces a single commit with the auto-fixes. Use proactively whenever the user says "clean up", "tidy this up", "run quality", "polish the code", or after a large feature lands and before a release. Guardrails: will refuse to run if `git status` shows uncommitted work on files outside the tool's target.
disable-model-invocation: true
---

# Rust Quality Sweep

User-only: Claude will not invoke this automatically. Type `/rust-quality-sweep` (or `/rust-quality-sweep src/search/`) to run.

## Why not just "cargo clippy --fix"?

`cargo clippy --fix -- -W clippy::pedantic` is unsafe in anger because pedantic flags subjective style lints that sometimes hurt readability, and a bulk auto-apply over a dirty tree is very hard to review. This skill separates **mechanical-safe** lints (apply automatically, easy to audit) from **judgment-needed** lints (surface for review).

## Preconditions (hard stop if any fail)

1. Working tree is clean in the scoped path — `git status --porcelain -- <path>` is empty. If not, refuse and ask the user to commit/stash first.
2. Current branch is NOT `main`. Create a branch (`chore/quality-sweep-<yyyymmdd>`) if on main.
3. `cargo check --all-targets` passes before we start. Never run auto-fixes on a broken tree.

## Safe-to-auto-apply lints

Apply via `cargo clippy --fix --allow-staged --allow-dirty -- -W <lint>` for each of these only:

- `clippy::needless_borrow`
- `clippy::redundant_clone`
- `clippy::unnecessary_wraps` (single-variant only; complex cases → manual list)
- `clippy::single_char_pattern`
- `clippy::unnecessary_cast`
- `clippy::needless_pass_by_value` (on `Copy` types only)
- `clippy::must_use_candidate` (adds `#[must_use]`)
- `clippy::redundant_field_names`

## Surface-for-review lints (DO NOT auto-apply)

These are judgment calls; collect into the report instead:

- `clippy::too_many_lines` — may want to refactor, may want to suppress
- `clippy::module_name_repetitions` — often a false positive on intentional naming
- `clippy::similar_names` — depends on context
- `clippy::missing_errors_doc` / `clippy::missing_panics_doc` — worth doing but better as a focused separate pass
- Any lint that suggests structural changes (e.g. "split this into a trait")

## Procedure

1. Enforce preconditions. Abort with a clear message on failure.
2. Run the safe-lint pass via a single `cargo clippy --fix --allow-staged -- <lint list>`.
3. `cargo check --all-targets && cargo test` — if either fails, `git reset --hard` and abort; surface the error.
4. Run a full `cargo clippy --all-targets -- -W clippy::pedantic -D warnings` and collect any remaining warnings for the surface-for-review block.
5. Scan for missing rustdoc on `pub` items: `cargo rustdoc --all-targets 2>&1 | grep "missing documentation"`. Add `///` stubs where the function name tells the full story ("Return callers of `target` (reverse-edge lookup). Cheap: O(degree)." format); skip items where a meaningful docstring needs real thought (add to review list).
6. Commit the safe changes with a clear message listing the lint names fixed and the count per lint.
7. Print the **Review list** — the surface-for-review lints the user might want to consider in a follow-up.

## Output

One commit on the current (non-main) branch, plus a terminal report:

```
Auto-fixed (N lints, M files):
  - clippy::redundant_clone: 7 hits in src/search/, src/edit/
  - clippy::must_use_candidate: 4 hits, added #[must_use]
  - ...

Review list (do NOT auto-apply, consider manually):
  - clippy::too_many_lines on src/parse/rust.rs::extract_visibility (138 lines)
  - missing docstring on pub fn my_api in src/graph/types.rs — consider what the invariants are
```

## Rollback

Single commit → single `git revert <sha>` if anything turns out bad. That's the whole point of the separation.
