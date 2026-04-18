# BlastGuard `apply_change` Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement BlastGuard's `apply_change` tool — edit a file, reparse it, emit up to four cascade warnings (SIGNATURE, ASYNC_CHANGE, ORPHAN, INTERFACE_BREAK), and return a bundled context (callers + tests) that saves the agent follow-up searches.

**Architecture:** A pure Rust library function `edit::apply_change(graph, session, request) -> ApplyChangeResponse`. The function performs four concurrent-in-spirit steps: (1) resolve and apply the edit to disk via `edit::apply` with fuzzy-match error recovery, (2) reparse the file, (3) diff old vs new symbol tables via `edit::diff`, (4) for each changed symbol fan out through `graph::impact` detectors and `search::structural::{callers_of, tests_for}` for bundled context. All four cascade detectors live in `src/graph/impact.rs` (where `Warning`/`WarningKind` already scaffold). Session state (`modified_files`, `modified_symbols`) is updated on success so `run_tests` (Plan 4) can attribute failures to recent edits. The MCP wire wrap with `#[tool]` annotations is Plan 4's job.

**Tech Stack:** Rust 1.82+. Reuses Plan 1's graph + parsers + session; Plan 2's search backends. New deps: `strsim` (already pinned at 0.11) for fuzzy `old_text` matching.

**Preconditions assumed by this plan:**
- Repo at `/home/adam/Documents/blastguard`. Branch: `phase-1-apply-change` from main (HEAD `a1a16c8`).
- `src/graph/impact.rs` has `Warning` + `WarningKind::{Signature,AsyncChange,Orphan,InterfaceBreak}` + `Warning::new(kind, symbol, body)` from Plan 1. Detectors are stubbed.
- `src/mcp/apply_change.rs` is a TODO-only stub.
- `src/session.rs::SessionState::{record_file_edit, record_symbol_edit, edits_ago}` exist.
- `src/search/structural::{callers_of, tests_for}` work per Plan 2.
- `src/error.rs::BlastGuardError` has `EditNotFound` and `AmbiguousEdit` variants (scaffolded in Plan 1).

**Definition of done:**
- `edit::apply_change(graph, session, req)` returns `Ok(ApplyChangeResponse)` for a valid edit; file on disk is updated; graph is reindexed for the edited file; warnings + context fire.
- Four cascade detectors emit correctly-formatted Warnings per SPEC §5.2.
- `isError: true` paths (file missing, old_text not found, ambiguous old_text, parse failure) surface as `BlastGuardError` that Plan 4's MCP handler can map to `CallToolResult { is_error: true, .. }`.
- Integration test end-to-end: edit a function's signature in the fixture, assert SIGNATURE warning fires and names the affected callers.
- `cargo check --all-targets && cargo test && cargo clippy --all-targets -- -W clippy::pedantic -D warnings && cargo build --release` — all green.
- Test count ≥ 220 (183 baseline after Plan 2 + ~35 new).

---

## File Structure

| Path | Responsibility |
|---|---|
| `src/edit/mod.rs` | Re-exports + the `apply_change` orchestrator entry point |
| `src/edit/apply.rs` | `apply_edit(file, old, new)` — disk I/O with exact-match then fuzzy fallback |
| `src/edit/diff.rs` | `SymbolDiff` — `added/removed/modified_sig/modified_body` classification |
| `src/edit/request.rs` | `ApplyChangeRequest` + `ApplyChangeResponse` DTO types (derive `Serialize`/`Deserialize` for Plan 4's MCP wire) |
| `src/edit/context.rs` | Bundled context builder — callers + tests for each changed symbol |
| `src/graph/impact.rs` | (existing) — implement `detect_signature`, `detect_async_change`, `detect_orphan`, `detect_interface_break` |
| `src/mcp/apply_change.rs` | (existing stub) — rewrite to export `handle_apply_change` thin wrapper around `edit::apply_change` |
| `tests/integration_apply_change.rs` | E2E: edit fixture, assert warnings + context |

Design notes:
- Split `edit/` from `graph/impact` because disk I/O + tree-sitter reparse are orthogonal to cascade detection. Impact detectors should take graph snapshots (old graph, new symbol) and be test-covered without touching the filesystem.
- `ApplyChangeRequest`/`Response` get their own file so Plan 4's MCP handler only imports one module.
- Context builder (`callers + tests`) is pulled out so it can be reused by `blastguard://status` (Plan 4) and future Phase 2 `around` queries.

---

## Task 1: DTO types — ApplyChangeRequest + ApplyChangeResponse

**Files:**
- Create: `src/edit/request.rs`
- Create: `src/edit/mod.rs`
- Modify: `src/lib.rs` (add `pub mod edit;`)

- [ ] **Step 1: Write the failing test**

Create `src/edit/request.rs`:

```rust
//! Request and response DTOs for the `apply_change` tool.
//!
//! Derives `Serialize`/`Deserialize` so Plan 4's rmcp `#[tool]` handler can
//! round-trip them over the MCP wire without a bridging layer.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::graph::impact::Warning;

/// One text replacement inside a single `apply_change` call. Matches a
/// single-shot `old_text → new_text` swap; the caller can submit multiple
/// in a single request to stage dependent edits atomically.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct Change {
    pub old_text: String,
    pub new_text: String,
}

/// Input to the `apply_change` MCP tool. Mirrors SPEC §3.2 exactly.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ApplyChangeRequest {
    pub file: PathBuf,
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub create_file: bool,
    #[serde(default)]
    pub delete_file: bool,
}

/// Response body for a successful `apply_change`. Error paths surface as
/// `BlastGuardError` from the caller and are rendered by Plan 4's MCP
/// handler with `isError: true`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ApplyChangeResponse {
    pub status: ApplyStatus,
    /// One-line summary, e.g. "Modified processRequest() in src/handler.ts. 2 cascade warnings."
    pub summary: String,
    pub warnings: Vec<Warning>,
    pub context: BundledContext,
}

/// Top-level status for an applied change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApplyStatus {
    Applied,
    Created,
    Deleted,
    /// No material change (whitespace/comments only).
    NoOp,
}

/// Pre-fetched follow-up data so the agent rarely needs another search.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct BundledContext {
    /// Inline caller snippets for each changed symbol — `file:line — signature`.
    pub callers: Vec<String>,
    /// Test files importing the edited file.
    pub tests: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_status_screaming_snake_serialisation() {
        assert_eq!(
            serde_json::to_string(&ApplyStatus::Applied).unwrap(),
            "\"APPLIED\""
        );
        assert_eq!(
            serde_json::to_string(&ApplyStatus::NoOp).unwrap(),
            "\"NO_OP\""
        );
    }

    #[test]
    fn request_round_trips_with_defaults() {
        let json = r#"{"file": "src/a.ts", "changes": []}"#;
        let req: ApplyChangeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.file, std::path::PathBuf::from("src/a.ts"));
        assert!(req.changes.is_empty());
        assert!(!req.create_file);
        assert!(!req.delete_file);
    }

    #[test]
    fn request_accepts_multiple_changes() {
        let json = r#"{
            "file": "src/a.ts",
            "changes": [
                {"old_text": "foo", "new_text": "bar"},
                {"old_text": "baz", "new_text": "qux"}
            ]
        }"#;
        let req: ApplyChangeRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.changes.len(), 2);
        assert_eq!(req.changes[0].old_text, "foo");
    }
}
```

Create `src/edit/mod.rs`:

```rust
//! `apply_change` tool backend — SPEC §3.2.
//!
//! Orchestrates: (1) disk edit via [`apply`], (2) reparse via [`crate::parse`],
//! (3) symbol diff via [`diff`], (4) cascade detection via
//! [`crate::graph::impact`], (5) bundled context via [`context`].
//!
//! Plan 4 wires the entry point [`apply_change`] into an rmcp `#[tool]`
//! handler; for now it returns a plain [`Result`] that the caller can map
//! into `CallToolResult { is_error: true, .. }` on failure.

pub mod apply;
pub mod context;
pub mod diff;
pub mod request;

pub use request::{ApplyChangeRequest, ApplyChangeResponse, ApplyStatus, BundledContext, Change};
```

Add stubs so the module compiles:

`src/edit/apply.rs`:
```rust
//! On-disk file edit primitive — Task 2.
```

`src/edit/diff.rs`:
```rust
//! Symbol-table diff — Task 5.
```

`src/edit/context.rs`:
```rust
//! Bundled context — Task 11.
```

Modify `src/lib.rs`: add `pub mod edit;` after `pub mod config;` (keep alphabetical order).

- [ ] **Step 2: Run tests to confirm red**

```bash
cd /home/adam/Documents/blastguard
cargo test -p blastguard edit::request::tests 2>&1 | tail -15
```
Expected: compile error because `Warning` needs to be exported from `graph::impact` as a `pub` type (Plan 1 already made it `pub` — verify with a grep if in doubt). If `graph::impact::Warning` is not `pub`, `pub use graph::impact::Warning` from `src/graph/mod.rs` is needed.

- [ ] **Step 3: If compile fails on a missing re-export, surface `Warning` + `WarningKind` from `src/graph/mod.rs`**

Open `src/graph/mod.rs` and add to the existing `pub use` block:
```rust
pub use impact::{Warning, WarningKind};
```

- [ ] **Step 4: Run tests green**

```bash
cargo test 2>&1 | grep "test result"
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
```
Expected: 183 → 186 (3 new DTO tests). Clippy clean.

If clippy fires on `clippy::module_name_repetitions` for `ApplyChangeRequest` / `ApplyChangeResponse`, `src/lib.rs` already has `#![allow(clippy::module_name_repetitions)]` — verify.

- [ ] **Step 5: Commit**

```bash
cd /home/adam/Documents/blastguard
git checkout -b phase-1-apply-change
git add src/edit/ src/lib.rs src/graph/mod.rs
git commit -m "phase 1.6: ApplyChangeRequest / Response DTOs + edit module split

Creates the edit/ module with request (DTOs), apply (disk edit — Task 2),
diff (symbol diff — Task 5), context (bundled context — Task 11)
sub-modules. ApplyChangeRequest mirrors SPEC §3.2 with file, changes,
create_file, delete_file. ApplyChangeResponse carries status (ApplyStatus
enum), summary, warnings (Vec<Warning>), context (BundledContext with
callers + tests). ApplyStatus serialises as SCREAMING_SNAKE_CASE."
```

---

## Task 2: File edit primitive — exact old_text match

**Files:**
- Modify: `src/edit/apply.rs`

- [ ] **Step 1: Write the failing test**

Replace `src/edit/apply.rs`:

```rust
//! On-disk file edit primitive.
//!
//! [`apply_edit`] performs one `old_text → new_text` swap in the target
//! file. If `old_text` appears exactly once, the swap succeeds. If it
//! doesn't appear or appears multiple times, this function returns an
//! error from [`crate::error::BlastGuardError`]; the `apply_change`
//! orchestrator maps those into `CallToolResult { is_error: true, .. }`.

use std::path::Path;

use crate::error::{BlastGuardError, Result};

/// Replace the single occurrence of `old_text` with `new_text` in `path`.
///
/// # Errors
/// - [`BlastGuardError::Io`] on read/write failure.
/// - [`BlastGuardError::EditNotFound`] when `old_text` doesn't appear
///   anywhere in the file (Task 3 adds fuzzy-match hints).
/// - [`BlastGuardError::AmbiguousEdit`] when `old_text` appears 2+ times
///   (Task 4 populates the line numbers).
pub fn apply_edit(path: &Path, old_text: &str, new_text: &str) -> Result<()> {
    let body = std::fs::read_to_string(path).map_err(|source| BlastGuardError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let occurrences = body.matches(old_text).count();
    match occurrences {
        0 => Err(BlastGuardError::EditNotFound {
            path: path.to_path_buf(),
            line: 0,
            similarity: 0.0,
            fragment: String::new(),
        }),
        1 => {
            let updated = body.replacen(old_text, new_text, 1);
            std::fs::write(path, updated).map_err(|source| BlastGuardError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            Ok(())
        }
        n => Err(BlastGuardError::AmbiguousEdit {
            path: path.to_path_buf(),
            count: n,
            lines: Vec::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_edit_exact_single_match_rewrites_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "fn foo() { return 1; }").expect("write");

        apply_edit(&path, "return 1", "return 2").expect("apply_edit");

        let after = std::fs::read_to_string(&path).expect("read");
        assert_eq!(after, "fn foo() { return 2; }");
    }

    #[test]
    fn apply_edit_missing_old_text_returns_edit_not_found() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "fn foo() {}").expect("write");
        let err = apply_edit(&path, "NOT_PRESENT", "x").expect_err("should error");
        assert!(matches!(err, BlastGuardError::EditNotFound { .. }), "got {err:?}");
    }

    #[test]
    fn apply_edit_ambiguous_old_text_returns_ambiguous_edit() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("a.ts");
        std::fs::write(&path, "a = 1\nb = 1\n").expect("write");
        let err = apply_edit(&path, "= 1", "= 2").expect_err("ambiguous");
        match err {
            BlastGuardError::AmbiguousEdit { count, .. } => assert_eq!(count, 2),
            e => panic!("wrong variant: {e:?}"),
        }
    }

    #[test]
    fn apply_edit_missing_file_returns_io_error() {
        let err = apply_edit(std::path::Path::new("/nope/does/not/exist"), "x", "y")
            .expect_err("should error");
        assert!(matches!(err, BlastGuardError::Io { .. }), "got {err:?}");
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test -p blastguard edit::apply::tests 2>&1 | tail -15
```
Expected: 4 passed.

- [ ] **Step 3: Clippy + commit**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/edit/apply.rs
git commit -m "phase 1.6: apply_edit — exact old_text match with missing/ambiguous errors"
```

---

## Task 3: Fuzzy fallback for EditNotFound

When `old_text` doesn't appear exactly, return the closest matching line (Levenshtein, similarity %) so the agent can re-issue the edit with the correct snippet.

**Files:**
- Modify: `src/edit/apply.rs`

- [ ] **Step 1: Test**

Add to `tests` module:
```rust
#[test]
fn edit_not_found_carries_closest_match_and_similarity() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("a.ts");
    std::fs::write(&path, "function processRequest(req) {\n    return handler(req);\n}\n")
        .expect("write");
    // Slightly wrong — no `req)` parameter.
    let err = apply_edit(&path, "function processRequest() {", "function x() {")
        .expect_err("not found");
    match err {
        BlastGuardError::EditNotFound { line, similarity, fragment, .. } => {
            assert_eq!(line, 1, "closest line should be 1 (the function header)");
            assert!(similarity >= 0.7, "similarity {similarity} too low for a near-miss");
            assert!(fragment.contains("processRequest"), "fragment = {fragment}");
        }
        e => panic!("wrong variant: {e:?}"),
    }
}
```

- [ ] **Step 2: Red**

```bash
cargo test -p blastguard edit::apply::tests::edit_not_found_carries_closest_match_and_similarity 2>&1 | tail -10
```

- [ ] **Step 3: Extend `apply_edit` to populate closest-match fields**

Replace the `0` match arm:
```rust
0 => {
    let (line, similarity, fragment) = closest_line(&body, old_text);
    Err(BlastGuardError::EditNotFound {
        path: path.to_path_buf(),
        line,
        similarity,
        fragment,
    })
}
```

Add the helper above `#[cfg(test)]`:
```rust
/// Scan `body` for the line with the highest similarity to `needle` under
/// normalised Levenshtein. Returns `(line_number_1_based, similarity_0_to_1, fragment)`.
fn closest_line(body: &str, needle: &str) -> (u32, f32, String) {
    let mut best_line: u32 = 0;
    let mut best_sim: f32 = 0.0;
    let mut best_fragment = String::new();
    for (idx, line) in body.lines().enumerate() {
        let dist = strsim::levenshtein(line, needle);
        let max_len = line.len().max(needle.len()).max(1);
        // Similarity = 1 - (dist / max_len).
        #[allow(clippy::cast_precision_loss)]
        let sim = 1.0_f32 - (dist as f32 / max_len as f32);
        if sim > best_sim {
            best_sim = sim;
            best_line = u32::try_from(idx)
                .unwrap_or(u32::MAX)
                .saturating_add(1);
            best_fragment = line.to_string();
        }
    }
    (best_line, best_sim, best_fragment)
}
```

- [ ] **Step 4: Green**

```bash
cargo test -p blastguard edit::apply::tests 2>&1 | tail -10
```
Expected: 5 passed.

- [ ] **Step 5: Clippy + commit**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/edit/apply.rs
git commit -m "phase 1.6: EditNotFound now carries closest line + similarity %"
```

---

## Task 4: AmbiguousEdit carries all match line numbers

**Files:** `src/edit/apply.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn ambiguous_edit_lists_all_match_line_numbers() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("a.ts");
    std::fs::write(&path, "a = 1\nb = 1\nc = 1\n").expect("write");
    let err = apply_edit(&path, "= 1", "= 2").expect_err("ambiguous");
    match err {
        BlastGuardError::AmbiguousEdit { count, lines, .. } => {
            assert_eq!(count, 3);
            assert_eq!(lines, vec![1, 2, 3]);
        }
        e => panic!("wrong variant: {e:?}"),
    }
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Populate `lines` in the ambiguous arm**

Replace the `n` arm:
```rust
n => {
    let lines = find_match_lines(&body, old_text);
    Err(BlastGuardError::AmbiguousEdit {
        path: path.to_path_buf(),
        count: n,
        lines,
    })
}
```

Add helper:
```rust
/// Enumerate 1-based line numbers where `needle` appears in `body`.
/// Multi-line needles count once per starting line.
fn find_match_lines(body: &str, needle: &str) -> Vec<u32> {
    let mut lines = Vec::new();
    let mut byte_offsets = Vec::new();
    let mut cursor = 0usize;
    while let Some(found) = body[cursor..].find(needle) {
        byte_offsets.push(cursor + found);
        cursor = cursor + found + needle.len().max(1);
    }
    for offset in byte_offsets {
        let line = body[..offset].chars().filter(|&c| c == '\n').count();
        let line_1based = u32::try_from(line)
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        lines.push(line_1based);
    }
    lines
}
```

- [ ] **Step 4: Green + commit**

```bash
cargo test -p blastguard edit::apply::tests 2>&1 | tail -5
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/edit/apply.rs
git commit -m "phase 1.6: AmbiguousEdit populates all 1-based match line numbers"
```

---

## Task 5: Symbol diff — classify changes

**Files:** `src/edit/diff.rs`

`SymbolDiff` classifies what changed between two symbol lists (pre-edit + post-edit) for the same file:

- `added` — in new, not in old (keyed by `SymbolId`).
- `removed` — in old, not in new.
- `modified_sig` — in both, signatures differ (params / return_type / is_async).
- `modified_body` — in both, signatures match but body_hash differs.

- [ ] **Step 1: Test**

Replace `src/edit/diff.rs`:

```rust
//! Symbol-table diff between a file's pre-edit and post-edit state.

use std::collections::HashMap;

use crate::graph::types::{Symbol, SymbolId};

/// Classification of changes between two symbol sets for the same file.
#[derive(Debug, Default)]
pub struct SymbolDiff {
    pub added: Vec<Symbol>,
    pub removed: Vec<Symbol>,
    pub modified_sig: Vec<(Symbol, Symbol)>,   // (old, new)
    pub modified_body: Vec<(Symbol, Symbol)>,  // (old, new)
}

impl SymbolDiff {
    /// `true` when every category is empty (whitespace/comments-only change).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.modified_sig.is_empty()
            && self.modified_body.is_empty()
    }
}

/// Diff two symbol lists by [`SymbolId`]. Signature equality is determined
/// by `signature` + `return_type` + `is_async`; body equality by `body_hash`.
#[must_use]
pub fn diff(old: &[Symbol], new: &[Symbol]) -> SymbolDiff {
    let old_by_id: HashMap<&SymbolId, &Symbol> = old.iter().map(|s| (&s.id, s)).collect();
    let new_by_id: HashMap<&SymbolId, &Symbol> = new.iter().map(|s| (&s.id, s)).collect();

    let mut out = SymbolDiff::default();

    for (id, s) in &new_by_id {
        match old_by_id.get(id) {
            None => out.added.push((*s).clone()),
            Some(old_sym) => {
                let sig_changed = old_sym.signature != s.signature
                    || old_sym.return_type != s.return_type
                    || old_sym.is_async != s.is_async
                    || old_sym.params != s.params;
                if sig_changed {
                    out.modified_sig.push(((*old_sym).clone(), (*s).clone()));
                } else if old_sym.body_hash != s.body_hash {
                    out.modified_body.push(((*old_sym).clone(), (*s).clone()));
                }
            }
        }
    }
    for (id, s) in &old_by_id {
        if !new_by_id.contains_key(id) {
            out.removed.push((*s).clone());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};
    use std::path::PathBuf;

    fn sym(name: &str, sig: &str, body_hash: u64, is_async: bool) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from("x.ts"),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 5,
            signature: sig.to_string(),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash,
            is_async,
            embedding_id: None,
        }
    }

    #[test]
    fn diff_detects_added() {
        let old = vec![sym("foo", "foo()", 0, false)];
        let new = vec![
            sym("foo", "foo()", 0, false),
            sym("bar", "bar()", 0, false),
        ];
        let d = diff(&old, &new);
        assert_eq!(d.added.len(), 1);
        assert_eq!(d.added[0].id.name, "bar");
        assert!(d.removed.is_empty());
    }

    #[test]
    fn diff_detects_removed() {
        let old = vec![
            sym("foo", "foo()", 0, false),
            sym("bar", "bar()", 0, false),
        ];
        let new = vec![sym("foo", "foo()", 0, false)];
        let d = diff(&old, &new);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.removed[0].id.name, "bar");
    }

    #[test]
    fn diff_detects_modified_sig() {
        let old = vec![sym("foo", "foo()", 0, false)];
        let new = vec![sym("foo", "foo(x: i32)", 0, false)];
        let d = diff(&old, &new);
        assert_eq!(d.modified_sig.len(), 1);
        assert!(d.modified_body.is_empty());
    }

    #[test]
    fn diff_detects_modified_body_only() {
        let old = vec![sym("foo", "foo()", 1, false)];
        let new = vec![sym("foo", "foo()", 2, false)];
        let d = diff(&old, &new);
        assert!(d.modified_sig.is_empty());
        assert_eq!(d.modified_body.len(), 1);
    }

    #[test]
    fn diff_detects_async_flip_as_modified_sig() {
        let old = vec![sym("foo", "foo()", 0, false)];
        let new = vec![sym("foo", "foo()", 0, true)];
        let d = diff(&old, &new);
        assert_eq!(d.modified_sig.len(), 1);
    }

    #[test]
    fn diff_empty_when_nothing_changed() {
        let old = vec![sym("foo", "foo()", 1, false)];
        let new = vec![sym("foo", "foo()", 1, false)];
        let d = diff(&old, &new);
        assert!(d.is_empty());
    }
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Verify green (implementation is in Step 1's file)**

```bash
cargo test -p blastguard edit::diff::tests 2>&1 | tail -10
```

- [ ] **Step 4: Commit**

```bash
git add src/edit/diff.rs
git commit -m "phase 1.6: symbol diff — added/removed/modified_sig/modified_body"
```

---

## Task 6: SIGNATURE cascade detector

**Files:** `src/graph/impact.rs`

- [ ] **Step 1: Test**

Add after `WarningKind::InterfaceBreak` definition:

```rust
#[cfg(test)]
mod detector_tests {
    use super::*;
    use crate::graph::types::{
        CodeGraph, Confidence, Edge, EdgeKind, Symbol, SymbolId, SymbolKind, Visibility,
    };
    use std::path::PathBuf;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 3,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    fn connect(g: &mut CodeGraph, from: &Symbol, to: &Symbol) {
        g.insert_edge(Edge {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Calls,
            line: 10,
            confidence: Confidence::Certain,
        });
    }

    #[test]
    fn signature_warning_lists_callers() {
        let mut g = CodeGraph::new();
        let target = sym("processRequest", "h.ts");
        let caller_a = sym("api", "api.ts");
        let caller_b = sym("admin", "admin.ts");
        g.insert_symbol(target.clone());
        g.insert_symbol(caller_a.clone());
        g.insert_symbol(caller_b.clone());
        connect(&mut g, &caller_a, &target);
        connect(&mut g, &caller_b, &target);

        let mut old = target.clone();
        old.signature = "processRequest(req, res)".to_string();
        let mut new = target.clone();
        new.signature = "processRequest(req, res, next)".to_string();

        let warning = detect_signature(&g, &old, &new).expect("should fire");
        assert_eq!(warning.kind, WarningKind::Signature);
        assert!(warning.body.contains("processRequest"), "body={}", warning.body);
        assert!(warning.body.contains("2 callers"), "body={}", warning.body);
        assert!(warning.body.contains("api.ts") || warning.body.contains("admin.ts"));
    }

    #[test]
    fn signature_warning_none_when_no_callers() {
        let mut g = CodeGraph::new();
        let target = sym("lonely", "x.ts");
        g.insert_symbol(target.clone());
        let mut new = target.clone();
        new.signature = "lonely(x)".to_string();
        assert!(detect_signature(&g, &target, &new).is_none());
    }
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement `detect_signature`**

Append to `src/graph/impact.rs` above the test module:

```rust
use crate::graph::ops::callers;
use crate::graph::types::{CodeGraph, Symbol};

/// SIGNATURE — function params / return-type / async-ness changed.
/// Fires when the modified symbol has ≥ 1 caller; body lists up to 10
/// caller file:line pairs (SPEC §5.4 cap).
#[must_use]
pub fn detect_signature(graph: &CodeGraph, old: &Symbol, new: &Symbol) -> Option<Warning> {
    let _ = old; // placeholder if we need delta rendering later
    let caller_ids = callers(graph, &new.id);
    if caller_ids.is_empty() {
        return None;
    }
    let total = caller_ids.len();
    let shown: Vec<String> = caller_ids
        .iter()
        .take(10)
        .map(|id| format!("{}:{}", id.file.display(), id.name))
        .collect();
    let more = if total > 10 {
        format!(" …and {} more ({} total)", total - 10, total)
    } else {
        String::new()
    };
    let body = format!(
        "{}() signature changed. {} callers may break: {}{}",
        new.id.name,
        total,
        shown.join(", "),
        more
    );
    Some(Warning::new(WarningKind::Signature, new.id.clone(), body))
}
```

- [ ] **Step 4: Green + commit**

```bash
cargo test -p blastguard graph::impact::detector_tests 2>&1 | tail -5
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/graph/impact.rs
git commit -m "phase 1.6: SIGNATURE cascade detector — lists callers of modified sig"
```

---

## Task 7: ASYNC_CHANGE cascade detector

Same fixture style, pin the sync↔async flip.

**Files:** `src/graph/impact.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn async_change_warning_when_function_becomes_async() {
    let mut g = CodeGraph::new();
    let target = sym("process", "h.ts");
    let caller = sym("api", "api.ts");
    g.insert_symbol(target.clone());
    g.insert_symbol(caller.clone());
    connect(&mut g, &caller, &target);

    let mut old = target.clone();
    old.is_async = false;
    let mut new = target.clone();
    new.is_async = true;

    let warning = detect_async_change(&g, &old, &new).expect("should fire");
    assert_eq!(warning.kind, WarningKind::AsyncChange);
    assert!(warning.body.contains("sync→async") || warning.body.contains("async"));
    assert!(warning.body.contains("1 caller"));
}

#[test]
fn async_change_warning_none_when_sync_stays_sync() {
    let mut g = CodeGraph::new();
    let target = sym("process", "h.ts");
    let caller = sym("api", "api.ts");
    g.insert_symbol(target.clone());
    g.insert_symbol(caller.clone());
    connect(&mut g, &caller, &target);
    assert!(detect_async_change(&g, &target, &target).is_none());
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
/// ASYNC_CHANGE — function flipped between sync and async. Callers that
/// don't `await` receive a Promise/Future instead of the value.
#[must_use]
pub fn detect_async_change(graph: &CodeGraph, old: &Symbol, new: &Symbol) -> Option<Warning> {
    if old.is_async == new.is_async {
        return None;
    }
    let caller_ids = callers(graph, &new.id);
    let direction = if new.is_async { "sync→async" } else { "async→sync" };
    let total = caller_ids.len();
    let body = format!(
        "{}() {}. {} caller{} need{} update.",
        new.id.name,
        direction,
        total,
        if total == 1 { "" } else { "s" },
        if total == 1 { "s" } else { "" }
    );
    Some(Warning::new(WarningKind::AsyncChange, new.id.clone(), body))
}
```

- [ ] **Step 4: Green + commit**

```bash
cargo test -p blastguard graph::impact::detector_tests 2>&1 | tail -5
git add src/graph/impact.rs
git commit -m "phase 1.6: ASYNC_CHANGE cascade detector"
```

---

## Task 8: ORPHAN cascade detector

Fires when a symbol was removed (Task 5's `removed` bucket) and still has callers — i.e., at least one forward edge exists with `to.file == removed.id.file && to.name == removed.id.name`. This works because `CodeGraph::remove_file` preserves caller forward edges (Plan 1 Task 0 fix).

**Files:** `src/graph/impact.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn orphan_warning_when_removed_symbol_has_callers() {
    let mut g = CodeGraph::new();
    let target = sym("gone", "h.ts");
    let caller = sym("api", "api.ts");
    g.insert_symbol(target.clone());
    g.insert_symbol(caller.clone());
    connect(&mut g, &caller, &target);

    // Simulate: target's file reindexed after delete. CodeGraph::remove_file
    // drops the symbol but keeps caller's forward edge.
    g.remove_file(std::path::Path::new("h.ts"));

    let warning = detect_orphan(&g, &target).expect("should fire");
    assert_eq!(warning.kind, WarningKind::Orphan);
    assert!(warning.body.contains("gone"));
    assert!(warning.body.contains("1 caller"));
}

#[test]
fn orphan_warning_none_when_no_callers_remaining() {
    let mut g = CodeGraph::new();
    let target = sym("gone", "h.ts");
    g.insert_symbol(target.clone());
    g.remove_file(std::path::Path::new("h.ts"));
    assert!(detect_orphan(&g, &target).is_none());
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
/// ORPHAN — a symbol was removed but callers' forward edges still point to
/// it. Scans `graph.forward_edges` for any edge whose `to` matches the
/// removed symbol's id. (Plan 1's remove_file preserves these dangling
/// edges for exactly this purpose.)
#[must_use]
pub fn detect_orphan(graph: &CodeGraph, removed: &Symbol) -> Option<Warning> {
    let dangling: Vec<&crate::graph::types::SymbolId> = graph
        .forward_edges
        .iter()
        .flat_map(|(_, edges)| edges.iter())
        .filter(|e| e.to == removed.id)
        .map(|e| &e.from)
        .collect();
    if dangling.is_empty() {
        return None;
    }
    let total = dangling.len();
    let shown: Vec<String> = dangling
        .iter()
        .take(10)
        .map(|id| format!("{}:{}", id.file.display(), id.name))
        .collect();
    let body = format!(
        "{}() removed but {} caller{} still reference it: {}",
        removed.id.name,
        total,
        if total == 1 { "" } else { "s" },
        shown.join(", ")
    );
    Some(Warning::new(WarningKind::Orphan, removed.id.clone(), body))
}
```

- [ ] **Step 4: Green + commit**

```bash
cargo test -p blastguard graph::impact::detector_tests 2>&1 | tail -5
git add src/graph/impact.rs
git commit -m "phase 1.6: ORPHAN cascade detector — dangling forward edges"
```

---

## Task 9: INTERFACE_BREAK cascade detector

Fires when a modified symbol is an `Interface` / `Trait` and its signature changed — implementing classes/structs are listed via `EdgeKind::Implements`.

**Files:** `src/graph/impact.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn interface_break_warning_lists_implementors() {
    let mut g = CodeGraph::new();
    let iface = Symbol {
        id: SymbolId {
            file: PathBuf::from("api.ts"),
            name: "Greeter".to_string(),
            kind: SymbolKind::Interface,
        },
        ..sym("Greeter", "api.ts")
    };
    let impl_a = sym("EnglishGreeter", "english.ts");
    g.insert_symbol(iface.clone());
    g.insert_symbol(impl_a.clone());
    g.insert_edge(Edge {
        from: impl_a.id.clone(),
        to: iface.id.clone(),
        kind: EdgeKind::Implements,
        line: 1,
        confidence: Confidence::Certain,
    });

    let mut old = iface.clone();
    old.signature = "interface Greeter { greet(): string }".to_string();
    let mut new = iface.clone();
    new.signature = "interface Greeter { greet(name: string): string }".to_string();

    let warning = detect_interface_break(&g, &old, &new).expect("should fire");
    assert_eq!(warning.kind, WarningKind::InterfaceBreak);
    assert!(warning.body.contains("Greeter"));
    assert!(warning.body.contains("EnglishGreeter") || warning.body.contains("1 impl"));
}

#[test]
fn interface_break_none_when_not_interface_or_trait() {
    let mut g = CodeGraph::new();
    let f = sym("foo", "f.ts");
    g.insert_symbol(f.clone());
    assert!(detect_interface_break(&g, &f, &f).is_none());
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
use crate::graph::types::{EdgeKind, SymbolKind};

/// INTERFACE_BREAK — a TS interface or Rust trait's signature changed.
/// Lists implementing classes/structs via reverse Implements edges.
#[must_use]
pub fn detect_interface_break(graph: &CodeGraph, old: &Symbol, new: &Symbol) -> Option<Warning> {
    if !matches!(new.id.kind, SymbolKind::Interface | SymbolKind::Trait) {
        return None;
    }
    if old.signature == new.signature {
        return None;
    }
    let implementors: Vec<&crate::graph::types::SymbolId> = graph
        .reverse_edges
        .get(&new.id)
        .into_iter()
        .flatten()
        .filter(|e| e.kind == EdgeKind::Implements)
        .map(|e| &e.from)
        .collect();
    if implementors.is_empty() {
        return None;
    }
    let total = implementors.len();
    let shown: Vec<String> = implementors
        .iter()
        .take(10)
        .map(|id| format!("{}:{}", id.file.display(), id.name))
        .collect();
    let body = format!(
        "{} contract changed. {} impl{} may violate: {}",
        new.id.name,
        total,
        if total == 1 { "" } else { "s" },
        shown.join(", ")
    );
    Some(Warning::new(WarningKind::InterfaceBreak, new.id.clone(), body))
}
```

- [ ] **Step 4: Green + commit**

```bash
cargo test -p blastguard graph::impact::detector_tests 2>&1 | tail -5
git add src/graph/impact.rs
git commit -m "phase 1.6: INTERFACE_BREAK cascade detector — implementor enumeration"
```

---

## Task 10: Warning body clamp + summary line format

Confirm SPEC §5.4 output rules are pinned by tests.

**Files:** `src/graph/impact.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn warning_body_clamped_to_200_chars() {
    let long_body = "x".repeat(500);
    let w = Warning::new(
        WarningKind::Signature,
        sym("x", "y.ts").id,
        long_body,
    );
    assert!(w.body.chars().count() <= 200);
    assert!(w.body.ends_with('…'));
}

/// Rendered summary line for the MCP response top (SPEC §5.4).
/// Example: "3 warnings: 1 SIGNATURE, 1 ASYNC_CHANGE, 1 ORPHAN".
#[must_use]
pub fn summary_line(warnings: &[Warning]) -> String {
    let mut counts: std::collections::BTreeMap<&'static str, usize> = std::collections::BTreeMap::new();
    for w in warnings {
        *counts.entry(w.kind.tag()).or_insert(0) += 1;
    }
    if warnings.is_empty() {
        return "0 warnings".to_string();
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(tag, n)| format!("{n} {tag}"))
        .collect();
    format!("{} warnings: {}", warnings.len(), parts.join(", "))
}

#[test]
fn summary_line_groups_warnings_by_kind() {
    let id = sym("x", "y.ts").id;
    let warnings = vec![
        Warning::new(WarningKind::Signature, id.clone(), "sig".into()),
        Warning::new(WarningKind::Orphan, id.clone(), "orph".into()),
        Warning::new(WarningKind::Signature, id.clone(), "sig2".into()),
    ];
    let s = summary_line(&warnings);
    assert!(s.starts_with("3 warnings"));
    assert!(s.contains("2 SIGNATURE"));
    assert!(s.contains("1 ORPHAN"));
}

#[test]
fn summary_line_zero_case() {
    assert_eq!(summary_line(&[]), "0 warnings");
}
```

- [ ] **Step 2: Red / Green**

```bash
cargo test -p blastguard graph::impact:: 2>&1 | tail -10
```

The `summary_line` function goes into `src/graph/impact.rs` alongside the detectors. The clamp test already passes because `Warning::new` clamps in Plan 1.

- [ ] **Step 3: Commit**

```bash
git add src/graph/impact.rs
git commit -m "phase 1.6: summary_line() + clamp regression test"
```

---

## Task 11: Bundled context builder

For each changed symbol, collect up to 5 callers (via `search::structural::callers_of` by name) and collect tests for the edited file (via `search::structural::tests_for`).

**Files:** `src/edit/context.rs`

- [ ] **Step 1: Test**

Replace `src/edit/context.rs`:

```rust
//! Bundled context for `apply_change` responses — callers + tests of the
//! symbols affected by an edit (SPEC §3.2 "Context bundle eliminates
//! follow-up searches").

use std::path::Path;

use crate::edit::request::BundledContext;
use crate::graph::types::{CodeGraph, Symbol};
use crate::search::structural::{callers_of, tests_for};

/// Build the bundled context for a set of changed symbols in `file`.
/// - `callers` caps at 10 entries total (across all changed symbols).
/// - `tests` is the set of test-path importers of `file`.
#[must_use]
pub fn build(graph: &CodeGraph, file: &Path, changed: &[Symbol]) -> BundledContext {
    let mut callers = Vec::new();
    let per_symbol_cap = 5;

    for sym in changed {
        let hits = callers_of(graph, &sym.id.name, per_symbol_cap);
        for hit in hits {
            let line_str = if let Some(sig) = hit.signature.as_deref() {
                format!("{}:{} — {}", hit.file.display(), hit.line, sig)
            } else {
                format!("{}:{}", hit.file.display(), hit.line)
            };
            callers.push(line_str);
            if callers.len() >= 10 {
                break;
            }
        }
        if callers.len() >= 10 {
            break;
        }
    }

    let tests = tests_for(graph, &file.to_string_lossy())
        .into_iter()
        .map(|h| h.file.to_string_lossy().to_string())
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();

    BundledContext { callers, tests }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{
        Confidence, Edge, EdgeKind, Symbol, SymbolId, SymbolKind, Visibility,
    };
    use std::path::PathBuf;

    fn sym(name: &str, file: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 10,
            line_end: 20,
            signature: format!("fn {name}(x: i32)"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    #[test]
    fn build_returns_callers_and_skips_absent_tests() {
        let mut g = CodeGraph::new();
        let target = sym("processRequest", "src/handler.ts");
        let caller = sym("api", "src/api.ts");
        g.insert_symbol(target.clone());
        g.insert_symbol(caller.clone());
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });

        let ctx = build(&g, Path::new("src/handler.ts"), &[target]);
        assert_eq!(ctx.callers.len(), 1);
        assert!(ctx.callers[0].contains("api.ts"));
        assert!(ctx.tests.is_empty());
    }

    #[test]
    fn build_deduplicates_test_files() {
        let mut g = CodeGraph::new();
        let target_id = SymbolId {
            file: PathBuf::from("src/handler.ts"),
            name: "handler".to_string(),
            kind: SymbolKind::Module,
        };
        let test_id = SymbolId {
            file: PathBuf::from("tests/handler.test.ts"),
            name: "t".to_string(),
            kind: SymbolKind::Module,
        };
        for id in [&target_id, &test_id] {
            let mut s = sym(&id.name, &id.file.to_string_lossy());
            s.id = id.clone();
            g.insert_symbol(s);
        }
        g.insert_edge(Edge {
            from: test_id.clone(),
            to: target_id.clone(),
            kind: EdgeKind::Imports,
            line: 1,
            confidence: Confidence::Certain,
        });

        let target = sym("handler", "src/handler.ts");
        let ctx = build(&g, Path::new("src/handler.ts"), &[target]);
        assert_eq!(ctx.tests.len(), 1);
        assert!(ctx.tests[0].contains(".test."));
    }
}
```

- [ ] **Step 2: Red / Green**

```bash
cargo test -p blastguard edit::context::tests 2>&1 | tail -10
```

- [ ] **Step 3: Commit**

```bash
git add src/edit/context.rs
git commit -m "phase 1.6: bundled context builder — callers + test files"
```

---

## Task 12: `apply_change` orchestrator

Orchestrate: apply each `Change` to disk, reparse the file, diff symbols, detect warnings, build context, update session, return response.

**Files:** `src/edit/mod.rs`

- [ ] **Step 1: Test skeleton**

At the top of `src/edit/mod.rs`, after the module declarations, add the orchestrator signature + a test that pins the happy path.

```rust
use std::path::Path;
use std::sync::Mutex;

use crate::error::Result;
use crate::graph::types::CodeGraph;
use crate::session::SessionState;

/// Entry point for the `apply_change` tool backend.
///
/// Orchestrates: apply the edit(s) to disk → reparse the file → diff the
/// symbol table → run cascade detectors → build bundled context → record
/// the edit into [`SessionState`].
///
/// # Errors
/// Any error from disk I/O or edit resolution (EditNotFound, AmbiguousEdit)
/// bubbles up verbatim. The MCP handler (Plan 4) maps them to
/// `CallToolResult { is_error: true, .. }`.
pub fn apply_change(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: ApplyChangeRequest,
) -> Result<ApplyChangeResponse> {
    apply::orchestrate(graph, session, project_root, request)
}

#[cfg(test)]
mod orchestrator_tests {
    use super::*;
    use crate::graph::impact::WarningKind;
    use crate::index::indexer::cold_index;

    #[test]
    fn signature_edit_fires_signature_warning() {
        // Seed a tempdir with a fixture: handler.ts defines processRequest,
        // api.ts calls it. Cold-index. Apply an edit that changes the
        // signature. Assert a SIGNATURE warning fires and the caller is
        // listed in bundled context.
        let tmp = tempfile::tempdir().expect("tempdir");
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(tmp.path())
            .status()
            .expect("git init");
        std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
        std::fs::write(
            tmp.path().join("src/handler.ts"),
            "export function processRequest(req) { return req; }\n",
        ).expect("write handler");
        std::fs::write(
            tmp.path().join("src/api.ts"),
            "import { processRequest } from \"./handler\";\nexport function api() { return processRequest({}); }\n",
        ).expect("write api");

        let graph = Mutex::new(cold_index(tmp.path()).expect("cold_index"));
        let session = Mutex::new(SessionState::new());

        let req = ApplyChangeRequest {
            file: tmp.path().join("src/handler.ts"),
            changes: vec![Change {
                old_text: "processRequest(req)".to_string(),
                new_text: "processRequest(req, res)".to_string(),
            }],
            create_file: false,
            delete_file: false,
        };

        let resp = apply_change(&graph, &session, tmp.path(), req).expect("apply_change");
        assert_eq!(resp.status, ApplyStatus::Applied);
        assert!(resp.warnings.iter().any(|w| w.kind == WarningKind::Signature),
            "expected SIGNATURE warning; got {:?}", resp.warnings);
        assert!(resp.context.callers.iter().any(|c| c.contains("api.ts")),
            "expected api.ts in context; got {:?}", resp.context.callers);
    }
}
```

- [ ] **Step 2: Red**

```bash
cargo test -p blastguard edit::orchestrator_tests 2>&1 | tail -15
```
Compile error — `apply::orchestrate` doesn't exist.

- [ ] **Step 3: Implement `apply::orchestrate`**

Append to `src/edit/apply.rs`:

```rust
use std::sync::Mutex;

use crate::edit::context;
use crate::edit::diff;
use crate::edit::request::{
    ApplyChangeRequest, ApplyChangeResponse, ApplyStatus, BundledContext,
};
use crate::graph::impact::{
    detect_async_change, detect_interface_break, detect_orphan, detect_signature, summary_line,
    Warning,
};
use crate::graph::types::{CodeGraph, Symbol};
use crate::parse::{detect_language, Language};
use crate::session::SessionState;

/// Shared orchestrator: apply → reparse → diff → detect → context → session.
pub fn orchestrate(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    _project_root: &Path,
    request: ApplyChangeRequest,
) -> Result<ApplyChangeResponse> {
    let file = request.file.clone();

    // Capture the pre-edit symbols for this file.
    let pre_edit_symbols: Vec<Symbol> = {
        let g = graph.lock().expect("graph lock");
        g.file_symbols
            .get(&file)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| g.symbols.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    };

    // Apply each change.
    for change in &request.changes {
        apply_edit(&file, &change.old_text, &change.new_text)?;
    }

    // Reparse and update the graph.
    let source = std::fs::read_to_string(&file).map_err(|source| BlastGuardError::Io {
        path: file.clone(),
        source,
    })?;
    let out = match detect_language(&file) {
        Some(Language::TypeScript) => crate::parse::typescript::extract(&file, &source),
        Some(Language::JavaScript) => crate::parse::javascript::extract(&file, &source),
        Some(Language::Python) => crate::parse::python::extract(&file, &source),
        Some(Language::Rust) => crate::parse::rust::extract(&file, &source),
        None => {
            // Non-indexed language — treat as a no-op for graph purposes.
            return Ok(ApplyChangeResponse {
                status: ApplyStatus::Applied,
                summary: format!("Edited {} (no graph impact — unsupported language)", file.display()),
                warnings: Vec::new(),
                context: BundledContext::default(),
            });
        }
    };

    let new_symbols = out.symbols.clone();
    {
        let mut g = graph.lock().expect("graph lock");
        g.remove_file(&file);
        for sym in out.symbols {
            g.insert_symbol(sym);
        }
        for edge in out.edges {
            g.insert_edge(edge);
        }
        g.library_imports.extend(out.library_imports);
    }

    // Diff.
    let d = diff::diff(&pre_edit_symbols, &new_symbols);

    // Run detectors against the graph (read-only).
    let mut warnings: Vec<Warning> = Vec::new();
    {
        let g = graph.lock().expect("graph lock");
        for (old, new) in &d.modified_sig {
            if let Some(w) = detect_signature(&g, old, new) {
                warnings.push(w);
            }
            if let Some(w) = detect_async_change(&g, old, new) {
                warnings.push(w);
            }
            if let Some(w) = detect_interface_break(&g, old, new) {
                warnings.push(w);
            }
        }
        for removed in &d.removed {
            if let Some(w) = detect_orphan(&g, removed) {
                warnings.push(w);
            }
        }
    }

    // Bundled context for the modified-sig changes (callers of what the
    // agent most likely just edited).
    let changed_for_context: Vec<Symbol> = d
        .modified_sig
        .iter()
        .map(|(_, new)| new.clone())
        .chain(d.modified_body.iter().map(|(_, new)| new.clone()))
        .collect();
    let context = {
        let g = graph.lock().expect("graph lock");
        context::build(&g, &file, &changed_for_context)
    };

    // Record into session state.
    {
        let mut s = session.lock().expect("session lock");
        s.record_file_edit(&file);
        for (_, new) in &d.modified_sig {
            s.record_symbol_edit(new.id.clone());
        }
        for (_, new) in &d.modified_body {
            s.record_symbol_edit(new.id.clone());
        }
    }

    let status = if d.is_empty() {
        ApplyStatus::NoOp
    } else {
        ApplyStatus::Applied
    };
    let summary = format!(
        "{} {}. {}.",
        match status {
            ApplyStatus::NoOp => "No-op edit in",
            _ => "Modified",
        },
        file.display(),
        summary_line(&warnings)
    );

    Ok(ApplyChangeResponse {
        status,
        summary,
        warnings,
        context,
    })
}
```

Add the `use crate::error::BlastGuardError;` import to the top of `src/edit/apply.rs` if not already present.

- [ ] **Step 4: Green**

```bash
cargo test -p blastguard edit::orchestrator_tests 2>&1 | tail -15
```

If the SIGNATURE test fails because the Task-2's TS driver placeholder kind of callee (`SymbolKind::Function`) doesn't match the edited `processRequest`'s actual kind, verify the `callers_of` name-based lookup is finding the right symbol. The `detect_signature` implementation uses `callers(graph, &new.id)` which looks up by id — if the id doesn't match what `api.ts`'s Calls edge points to (e.g., kind mismatch), no callers will be found. Fix: relax the edge-target lookup in detect_signature to match by `(file, name)` ignoring kind (the Plan 2 final-review note about `to.kind = Function` placeholder).

If that fix is needed, add to `detect_signature`:
```rust
let by_name: Vec<&crate::graph::types::SymbolId> = graph
    .forward_edges
    .iter()
    .flat_map(|(_, edges)| edges.iter())
    .filter(|e| e.to.file == new.id.file && e.to.name == new.id.name)
    .map(|e| &e.from)
    .collect();
```
Use `by_name` instead of `callers(...)`. Apply the same pattern to `detect_async_change` if the async test also fails.

- [ ] **Step 5: Commit**

```bash
git add src/edit/
git commit -m "phase 1.6: apply_change orchestrator

Sequence: snapshot pre-edit symbols → apply edit to disk → reparse →
remove_file/re-insert in graph → diff old vs new → run 4 detectors over
modified/removed → build bundled context (callers + tests) → record
session state → return ApplyChangeResponse. Graph + session guarded by
Mutex so Plan 4's rmcp handler can pass shared state in."
```

---

## Task 13: create_file / delete_file flags

**Files:** `src/edit/apply.rs`

- [ ] **Step 1: Test**

```rust
#[test]
fn create_file_writes_new_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file = tmp.path().join("src/new.ts");
    let graph = Mutex::new(CodeGraph::new());
    let session = Mutex::new(SessionState::new());

    let req = ApplyChangeRequest {
        file: file.clone(),
        changes: vec![Change {
            old_text: String::new(),
            new_text: "export function fresh() {}\n".to_string(),
        }],
        create_file: true,
        delete_file: false,
    };

    let resp = orchestrate(&graph, &session, tmp.path(), req).expect("create");
    assert_eq!(resp.status, ApplyStatus::Created);
    assert!(file.is_file());
}

#[test]
fn delete_file_removes_disk_and_graph_entries() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let file = tmp.path().join("src/gone.ts");
    std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
    std::fs::write(&file, "export function doomed() {}\n").expect("write");

    let mut g = CodeGraph::new();
    g.insert_symbol(Symbol {
        id: crate::graph::types::SymbolId {
            file: file.clone(),
            name: "doomed".into(),
            kind: crate::graph::types::SymbolKind::Function,
        },
        line_start: 1, line_end: 1,
        signature: "doomed()".into(),
        params: vec![], return_type: None,
        visibility: crate::graph::types::Visibility::Export,
        body_hash: 0, is_async: false, embedding_id: None,
    });
    let graph = Mutex::new(g);
    let session = Mutex::new(SessionState::new());

    let req = ApplyChangeRequest {
        file: file.clone(),
        changes: Vec::new(),
        create_file: false,
        delete_file: true,
    };

    let resp = orchestrate(&graph, &session, tmp.path(), req).expect("delete");
    assert_eq!(resp.status, ApplyStatus::Deleted);
    assert!(!file.exists());
    let g = graph.lock().expect("lock");
    assert!(g.file_symbols.get(&file).is_none());
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Extend `orchestrate` to branch on flags**

Before the existing logic in `orchestrate`, add:

```rust
if request.create_file {
    if let Some(parent) = request.file.parent() {
        std::fs::create_dir_all(parent).map_err(|source| BlastGuardError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let content = request
        .changes
        .first()
        .map(|c| c.new_text.clone())
        .unwrap_or_default();
    std::fs::write(&request.file, content).map_err(|source| BlastGuardError::Io {
        path: request.file.clone(),
        source,
    })?;
    // Reparse fresh file into the graph (reuse existing parse path by
    // falling through to the normal orchestrate with the `Applied` path
    // overridden to `Created` at the end).
    // Simplest: recurse with create_file=false, then rewrite status.
    let mut inner = request.clone();
    inner.create_file = false;
    inner.changes.clear();
    let mut resp = orchestrate(graph, session, _project_root, inner)?;
    resp.status = ApplyStatus::Created;
    return Ok(resp);
}

if request.delete_file {
    std::fs::remove_file(&request.file).map_err(|source| BlastGuardError::Io {
        path: request.file.clone(),
        source,
    })?;
    {
        let mut g = graph.lock().expect("graph lock");
        g.remove_file(&request.file);
    }
    {
        let mut s = session.lock().expect("session lock");
        s.record_file_edit(&request.file);
    }
    return Ok(ApplyChangeResponse {
        status: ApplyStatus::Deleted,
        summary: format!("Deleted {}", request.file.display()),
        warnings: Vec::new(),
        context: BundledContext::default(),
    });
}
```

Note: `ApplyChangeRequest` must derive `Clone` (it does — see Task 1). If the recursion pattern trips borrow-check, inline the reparse directly instead.

- [ ] **Step 4: Green + commit**

```bash
cargo test -p blastguard edit::apply::tests 2>&1 | tail -10
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/edit/apply.rs
git commit -m "phase 1.6: create_file + delete_file flag handling"
```

---

## Task 14: mcp/apply_change.rs thin wrapper

**Files:** `src/mcp/apply_change.rs`

- [ ] **Step 1: Replace stub**

```rust
//! `apply_change` MCP tool handler.
//!
//! Plan 4 will wrap this in an rmcp `#[tool]` macro on the server struct;
//! for now the module exports a simple function that Plan 4 can adapt.

use std::path::Path;
use std::sync::Mutex;

use crate::edit::{apply_change, ApplyChangeRequest, ApplyChangeResponse};
use crate::error::Result;
use crate::graph::types::CodeGraph;
use crate::session::SessionState;

/// Thin pass-through — exists so Plan 4's MCP wiring has a stable symbol
/// to import from `crate::mcp::apply_change::handle`.
///
/// # Errors
/// Bubbles any error from the orchestrator; Plan 4's `#[tool]` adapter
/// maps those into `CallToolResult { is_error: true, .. }`.
pub fn handle(
    graph: &Mutex<CodeGraph>,
    session: &Mutex<SessionState>,
    project_root: &Path,
    request: ApplyChangeRequest,
) -> Result<ApplyChangeResponse> {
    apply_change(graph, session, project_root, request)
}
```

- [ ] **Step 2: Commit**

```bash
git add src/mcp/apply_change.rs
git commit -m "phase 1.6: mcp::apply_change::handle pass-through for Plan 4 wiring"
```

---

## Task 15: Integration test — end-to-end cascade

**Files:** `tests/integration_apply_change.rs`

- [ ] **Step 1: Write**

```rust
//! End-to-end: seed a small project in a tempdir, cold-index it, apply a
//! signature-changing edit, assert SIGNATURE fires + bundled context
//! names the caller.

use std::sync::Mutex;

use blastguard::edit::{apply_change, ApplyChangeRequest, ApplyStatus, Change};
use blastguard::graph::impact::WarningKind;
use blastguard::index::indexer::cold_index;
use blastguard::session::SessionState;

#[test]
fn signature_edit_end_to_end_cascade() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // git init so .gitignore would work if we added one.
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(tmp.path())
        .status()
        .expect("git init");
    std::fs::create_dir_all(tmp.path().join("src")).expect("mkdir");
    std::fs::write(
        tmp.path().join("src/handler.ts"),
        "export function processRequest(req) { return req; }\n",
    ).expect("write handler");
    std::fs::write(
        tmp.path().join("src/api.ts"),
        "import { processRequest } from \"./handler\";\nexport function api() { return processRequest({}); }\n",
    ).expect("write api");

    let graph = Mutex::new(cold_index(tmp.path()).expect("cold_index"));
    let session = Mutex::new(SessionState::new());

    let req = ApplyChangeRequest {
        file: tmp.path().join("src/handler.ts"),
        changes: vec![Change {
            old_text: "processRequest(req)".to_string(),
            new_text: "processRequest(req, res)".to_string(),
        }],
        create_file: false,
        delete_file: false,
    };

    let resp = apply_change(&graph, &session, tmp.path(), req).expect("apply");
    assert_eq!(resp.status, ApplyStatus::Applied);
    assert!(
        resp.warnings.iter().any(|w| w.kind == WarningKind::Signature),
        "expected SIGNATURE; got {:?}",
        resp.warnings
    );
    assert!(
        resp.context.callers.iter().any(|c| c.contains("api.ts")),
        "expected api.ts in callers; got {:?}",
        resp.context.callers
    );
}
```

- [ ] **Step 2: Run**

```bash
cargo test --test integration_apply_change 2>&1 | tail -10
```

If the test fails on `callers_of` not finding the caller (because TS driver's `to.kind = Function` placeholder vs actual kind mismatch in the HashMap lookup), the fix from Task 12 Step 4's "by_name" helper applies here too — the detector needs to look up by `(file, name)` ignoring kind.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_apply_change.rs
git commit -m "phase 1.6: integration test — signature edit cascade end-to-end"
```

---

## Task 16: Final verification gate

- [ ] **Step 1: Gates**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -3
cargo test 2>&1 | grep "test result"
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
cargo build --release 2>&1 | tail -3
```

Expected: 183 → ~220 tests passing (Tasks 1-15 introduce ~35 new tests). Clippy clean. Release build green.

- [ ] **Step 2: Commit**

```bash
git commit --allow-empty -m "phase 1.6: verification gate — apply_change complete

All four gates green. apply_change orchestrator live with four cascade
detectors (SIGNATURE, ASYNC_CHANGE, ORPHAN, INTERFACE_BREAK), symbol
diff, bundled context (callers + tests), session state update, and
create_file/delete_file flags. mcp::apply_change::handle is the pass-
through Plan 4 will wire into the rmcp #[tool] adapter.

Closes docs/superpowers/plans/2026-04-18-blastguard-phase-1-apply-change.md.
Next: Plan 4 (run_tests + rmcp wiring + watcher + benchmark)."
```

- [ ] **Step 3: Hand off to finishing-a-development-branch**

---

## Self-Review

**Spec coverage:**
- SPEC §3.2 apply_change description — Task 1 DTO + Task 12 orchestrator ✓
- SPEC §5.1 symbol diffing — Task 5 ✓
- SPEC §5.2 four cascade warnings — Tasks 6-9 ✓
- SPEC §5.4 output rules (200-char cap, summary line) — Task 10 ✓
- SPEC §3.2 bundled context (callers + tests) — Task 11 ✓
- SPEC §3.2 "writes immediately, no pending/confirm gate" — orchestrator writes before detecting (Task 12) ✓
- SPEC §3.2 create_file / delete_file — Task 13 ✓
- SPEC §3.5 isError paths — BlastGuardError variants already carry the data (EditNotFound with line/similarity/fragment, AmbiguousEdit with lines list); Plan 4's MCP adapter renders them ✓
- SPEC §4 session state update — Task 12 records modified_files and modified_symbols ✓

**Forward-compatibility gap noted during self-review:** Task 12's "Caller-kind placeholder" caveat — `detect_signature` + `detect_async_change` look up callers via `graph::ops::callers(graph, &new.id)` which uses SymbolId as a HashMap key. Plan 2's final review flagged that `to.kind = Function` placeholder from the TS driver may not match the real callee's kind. If integration tests expose this, Task 12 Step 4 gives the fix — match by `(file, name)` ignoring kind.

**Placeholder scan:** no "TBD" / "implement later" markers. Every step has runnable code.

**Type consistency:** `ApplyChangeRequest { file, changes, create_file, delete_file }` stable across Tasks 1, 12, 13, 15. `Warning { kind, symbol, body }` from Plan 1 used throughout. `SymbolDiff` fields (`added/removed/modified_sig/modified_body`) consumed by Task 12.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-18-blastguard-phase-1-apply-change.md`. Two execution options:

**1. Subagent-Driven (recommended)** — same pattern that got Plans 1 and 2 through cleanly.
**2. Inline Execution** — run in this session.

Which approach?
