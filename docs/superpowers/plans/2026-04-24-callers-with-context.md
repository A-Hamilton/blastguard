# `callers of NAME with context` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a BlastGuard query variant `callers of NAME with context` that returns each caller's enclosing statement via tree-sitter so agents can see argument values without a follow-up `read_file`.

**Architecture:** Additive extension to existing `QueryKind::Callers` (grows from `(String)` to `(String, bool)` where the bool is `with_context`). A new `src/search/context_extract.rs` module owns the AST extraction using the same thread-local tree-sitter parser pattern that `src/parse/rust.rs` already uses. `SearchHit` gains an optional `context` field rendered with a `  | ` prefix so consumers of today's compact-line format see no changes on the without-context path.

**Tech Stack:** Rust 1.79+, existing tree-sitter crates (`tree_sitter`, `tree_sitter_rust`, `tree_sitter_python`, `tree_sitter_typescript`), existing `thread_local!` parser pattern. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-04-24-callers-with-context-design.md`.

---

## File Structure

- **Modify `src/search/query.rs`** — `QueryKind::Callers` grows to `(String, bool)`; regex captures optional `with context` suffix; 2 new classifier tests. Existing tests updated for the new tuple shape.
- **Modify `src/search/dispatcher.rs`** — destructure the new tuple; pass `with_context` flag through to `structural::callers_of`. One-line change.
- **Create `src/search/context_extract.rs`** — new focused module with a single public function `enclosing_statement(file, line) -> Option<String>`. Thread-local parsers per language (Rust/Python/TS/JS), AST-climb to statement ancestor, fall back to ±1 line window. Unit tests colocated.
- **Modify `src/search/structural.rs`** — `callers_of` gains a `with_context: bool` param; when true, post-process each hit to attach context. 4 new tests.
- **Modify `src/search/hit.rs`** — `SearchHit` gains `context: Option<String>` field; `to_compact_line` renders context with `  | ` prefix after the signature. 2 new tests.
- **Modify `src/search/mod.rs`** — declare the new `context_extract` module.
- **Modify `bench/prompts.py::BLASTGUARD_BIAS`** — document the new query variant in the cheat-sheet. Tiny edit.

Single-plan scope: ~300 LOC + tests across 5 Rust files + 1 Python prompt file. No breaking changes; all existing `Callers(name)` callers just become `Callers(name, false)`.

---

## Task 1: Grow `QueryKind::Callers` to carry `with_context`

**Files:**
- Modify: `src/search/query.rs`

This task updates the enum shape and classifier to match the spec. The existing `callers of NAME` continues to work unchanged (gets `with_context=false`); the new `callers of NAME with context` variant gets `true`.

- [ ] **Step 1: Write the failing tests**

In `src/search/query.rs` inside `#[cfg(test)] mod tests`, add these tests (alongside the existing `callers_of_pattern` test):

```rust
    #[test]
    fn callers_of_pattern_without_context_flag() {
        assert_eq!(
            classify("callers of processRequest"),
            QueryKind::Callers("processRequest".into(), false)
        );
    }

    #[test]
    fn callers_of_pattern_with_context_flag() {
        assert_eq!(
            classify("callers of apply_edit with context"),
            QueryKind::Callers("apply_edit".into(), true)
        );
    }

    #[test]
    fn what_calls_alias_no_context_support() {
        // The `what calls X` alias doesn't accept `with context` — keeping
        // the two forms distinct. `what calls X with context` is treated
        // as a literal name to match existing laxness on `what calls`.
        assert_eq!(
            classify("what calls handler"),
            QueryKind::Callers("handler".into(), false)
        );
    }
```

Also update the **existing** `callers_of_pattern` and `what_calls_alias` tests in the same file to match the new `(String, bool)` shape — both existing assertions need `false` added:

```rust
    // Replace existing `callers_of_pattern` body:
    #[test]
    fn callers_of_pattern() {
        assert_eq!(
            classify("callers of processRequest"),
            QueryKind::Callers("processRequest".into(), false)
        );
    }

    // Replace existing `what_calls_alias` body:
    #[test]
    fn what_calls_alias() {
        assert_eq!(
            classify("what calls handler"),
            QueryKind::Callers("handler".into(), false)
        );
    }

    // Same for `callers_of_whitespace_in_arg` (near the bottom of the
    // tests module) — update its expected value:
    #[test]
    fn callers_of_whitespace_in_arg() {
        assert_eq!(
            classify("callers of some symbol"),
            QueryKind::Callers("some symbol".into(), false)
        );
    }

    // Also update `leading_trailing_whitespace_trimmed`:
    #[test]
    fn leading_trailing_whitespace_trimmed() {
        assert_eq!(
            classify("  callers of foo  "),
            QueryKind::Callers("foo".into(), false)
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p blastguard search::query::tests`
Expected: compile failure — `QueryKind::Callers` currently takes 1 arg not 2.

- [ ] **Step 3: Implement the enum + classifier changes**

In `src/search/query.rs`, change the enum variant:

```rust
// Before (around line 17):
Callers(String),
// After:
/// `callers of X` (with_context=false) / `callers of X with context` (with_context=true).
Callers(String, bool),
```

Update the classifier. Find the existing regex branch for `callers of`:

```rust
    if let Some(caps) = re(r"^(?:callers of|what calls)\s+(.+)$").captures(q) {
        return QueryKind::Callers(caps[1].trim().to_string());
    }
```

Replace with a two-step approach so only `callers of …` supports the suffix (the `what calls` alias stays strict):

```rust
    if let Some(caps) = re(r"^callers of\s+(.+?)(\s+with\s+context)?$").captures(q) {
        let name = caps[1].trim().to_string();
        let with_context = caps.get(2).is_some();
        return QueryKind::Callers(name, with_context);
    }
    if let Some(caps) = re(r"^what calls\s+(.+)$").captures(q) {
        return QueryKind::Callers(caps[1].trim().to_string(), false);
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p blastguard search::query::tests`
Expected: all tests pass (the 4 updated existing + 3 new).

- [ ] **Step 5: Update all non-test callsites of `QueryKind::Callers`**

There are only 2 construction sites outside tests: the classifier (already updated) and the dispatcher match arm. Also 4 test assertions that we just updated.

Search for any remaining 1-arg usage:

```bash
cargo check --all-targets 2>&1 | grep -E "Callers|error\[" | head -20
```

If `cargo check` flags any untouched site, fix those one by one. The only non-test expected compiler error is `src/search/dispatcher.rs:42` — fix in Task 2.

- [ ] **Step 6: Commit**

```bash
git add src/search/query.rs
git commit -m "search/query: QueryKind::Callers carries with_context flag

Grows the variant from (String) to (String, bool). Classifier regex
now captures an optional \`with context\` suffix on the \`callers of\`
form. \`what calls\` alias stays strict (single arg). All existing
tests updated to match the new tuple shape.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Wire `with_context` through the dispatcher

**Files:**
- Modify: `src/search/dispatcher.rs:42-44`

The dispatcher's match arm destructures `QueryKind::Callers(name)` — needs to destructure both fields and pass the bool through.

- [ ] **Step 1: Update the dispatcher**

In `src/search/dispatcher.rs`, find:

```rust
        QueryKind::Callers(name) => {
            structural::callers_of(graph, &name, CALLERS_MAX_HITS, project_root)
        }
```

Replace with:

```rust
        QueryKind::Callers(name, with_context) => {
            structural::callers_of(
                graph,
                &name,
                CALLERS_MAX_HITS,
                project_root,
                with_context,
            )
        }
```

Note: `callers_of` doesn't take a `with_context` param yet — this line will fail to compile until Task 4 lands. That's deliberate TDD: the plan's next few tasks drive that parameter through.

- [ ] **Step 2: Run cargo check to confirm the only compile error is in structural.rs**

Run: `cargo check --all-targets 2>&1 | tail -10`
Expected: error about `callers_of` not taking 5 args. No other errors.

- [ ] **Step 3: Commit (red state, intentional)**

```bash
git add src/search/dispatcher.rs
git commit -m "search/dispatcher: pass with_context through to callers_of

Red-state commit: dispatcher now passes the with_context flag
captured by the classifier. Next task grows callers_of's signature
to accept it.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Create `context_extract` module with AST-smart extraction

**Files:**
- Create: `src/search/context_extract.rs`
- Modify: `src/search/mod.rs` (declare the new module)

The hardest task. Builds the tree-sitter context extractor as a standalone, fully tested module before `structural.rs` wires it in.

- [ ] **Step 1: Declare the module**

In `src/search/mod.rs`, find the `pub mod` declarations and add:

```rust
pub(crate) mod context_extract;
```

Keep it `pub(crate)` — only `structural::callers_of` should call it.

- [ ] **Step 2: Write the module skeleton with failing tests**

Create `src/search/context_extract.rs`:

```rust
//! AST-smart context extraction around a call site — returns the
//! enclosing statement's text via tree-sitter so `callers of X with
//! context` can surface argument values without a follow-up read_file.
//!
//! See `docs/superpowers/specs/2026-04-24-callers-with-context-design.md`.

use std::path::Path;

/// Maximum context lines to return per hit. Guards against pathological
/// multi-line chains (e.g. hundred-line builder calls). Most enclosing
/// statements are 1-5 lines.
const MAX_CONTEXT_LINES: usize = 20;

/// Maximum ancestor hops before giving up and falling back to the
/// line-window heuristic. Tree-sitter ASTs are typically shallow
/// around a call expression; 8 covers realistic nesting.
const MAX_ANCESTOR_HOPS: usize = 8;

/// Return the enclosing statement's text around a call at `line` in
/// `file`. Best-effort: returns `None` only when the file is
/// unreadable or the language isn't supported by any of our
/// tree-sitter parsers. When the AST heuristic can't find a
/// statement ancestor, falls back to a ±1 line window.
///
/// `line` is 1-based (matching `Edge.line` in the graph).
#[must_use]
pub fn enclosing_statement(file: &Path, line: u32) -> Option<String> {
    let source = std::fs::read_to_string(file).ok()?;
    let language = detect_language(file)?;
    if let Some(text) = extract_via_ast(&source, line, language) {
        return Some(text);
    }
    Some(line_window_fallback(&source, line, 1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Tsx,
}

fn detect_language(file: &Path) -> Option<Language> {
    match file.extension().and_then(|e| e.to_str()) {
        Some("rs") => Some(Language::Rust),
        Some("py") => Some(Language::Python),
        Some("ts") => Some(Language::TypeScript),
        Some("tsx") => Some(Language::Tsx),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(Language::JavaScript),
        _ => None,
    }
}

/// Return a ±N-line window around `line` (1-based). Always succeeds
/// for any valid source string; clamps at file boundaries. Returns
/// at most MAX_CONTEXT_LINES lines.
fn line_window_fallback(source: &str, line: u32, radius: u32) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let center = (line.saturating_sub(1)) as usize;
    let start = center.saturating_sub(radius as usize);
    let end = (center + radius as usize + 1).min(lines.len());
    let slice = &lines[start..end];
    let capped = if slice.len() > MAX_CONTEXT_LINES {
        &slice[..MAX_CONTEXT_LINES]
    } else {
        slice
    };
    capped.join("\n")
}

// Tree-sitter extraction lives below. Each language owns a
// thread-local Parser (Parser is not Send-safe, matching
// src/parse/rust.rs's pattern).

fn extract_via_ast(source: &str, line: u32, lang: Language) -> Option<String> {
    match lang {
        Language::Rust => extract_rust(source, line),
        Language::Python => extract_python(source, line),
        Language::TypeScript => extract_typescript(source, line),
        Language::Tsx => extract_tsx(source, line),
        Language::JavaScript => extract_javascript(source, line),
    }
}

// ── Rust ──────────────────────────────────────────────────────────────

thread_local! {
    static RS_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_rust::language())
            .expect("tree-sitter Rust grammar must load");
        p
    });
}

fn extract_rust(source: &str, line: u32) -> Option<String> {
    RS_PARSER.with(|cell| {
        let tree = cell.borrow_mut().parse(source, None)?;
        let call = deepest_call_at_line(&tree.root_node(), line)?;
        let stmt = climb_to_statement(call, RUST_STATEMENT_KINDS)?;
        extract_node_text(source, stmt)
    })
}

const RUST_STATEMENT_KINDS: &[&str] = &[
    "let_declaration",
    "expression_statement",
    "return_expression",
    "macro_invocation",
    "assignment_expression",
];

// ── Python ────────────────────────────────────────────────────────────

thread_local! {
    static PY_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_python::language())
            .expect("tree-sitter Python grammar must load");
        p
    });
}

fn extract_python(source: &str, line: u32) -> Option<String> {
    PY_PARSER.with(|cell| {
        let tree = cell.borrow_mut().parse(source, None)?;
        let call = deepest_call_at_line(&tree.root_node(), line)?;
        let stmt = climb_to_statement(call, PY_STATEMENT_KINDS)?;
        extract_node_text(source, stmt)
    })
}

const PY_STATEMENT_KINDS: &[&str] = &[
    "expression_statement",
    "assignment",
    "return_statement",
    "if_statement",
];

// ── TypeScript ────────────────────────────────────────────────────────

thread_local! {
    static TS_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_typescript::language_typescript())
            .expect("tree-sitter TypeScript grammar must load");
        p
    });
}

fn extract_typescript(source: &str, line: u32) -> Option<String> {
    TS_PARSER.with(|cell| {
        let tree = cell.borrow_mut().parse(source, None)?;
        let call = deepest_call_at_line(&tree.root_node(), line)?;
        let stmt = climb_to_statement(call, TS_STATEMENT_KINDS)?;
        extract_node_text(source, stmt)
    })
}

thread_local! {
    static TSX_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_typescript::language_tsx())
            .expect("tree-sitter TSX grammar must load");
        p
    });
}

fn extract_tsx(source: &str, line: u32) -> Option<String> {
    TSX_PARSER.with(|cell| {
        let tree = cell.borrow_mut().parse(source, None)?;
        let call = deepest_call_at_line(&tree.root_node(), line)?;
        let stmt = climb_to_statement(call, TS_STATEMENT_KINDS)?;
        extract_node_text(source, stmt)
    })
}

const TS_STATEMENT_KINDS: &[&str] = &[
    "lexical_declaration",
    "expression_statement",
    "return_statement",
    "assignment_expression",
    "variable_declaration",
];

// ── JavaScript ────────────────────────────────────────────────────────

thread_local! {
    static JS_PARSER: std::cell::RefCell<tree_sitter::Parser> = std::cell::RefCell::new({
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_javascript::language())
            .expect("tree-sitter JavaScript grammar must load");
        p
    });
}

fn extract_javascript(source: &str, line: u32) -> Option<String> {
    JS_PARSER.with(|cell| {
        let tree = cell.borrow_mut().parse(source, None)?;
        let call = deepest_call_at_line(&tree.root_node(), line)?;
        let stmt = climb_to_statement(call, TS_STATEMENT_KINDS)?;
        extract_node_text(source, stmt)
    })
}

// ── Shared helpers ────────────────────────────────────────────────────

/// Walk the tree and return the deepest call-expression-ish node whose
/// start row is `line - 1` (0-indexed).
fn deepest_call_at_line<'t>(
    root: &tree_sitter::Node<'t>,
    line: u32,
) -> Option<tree_sitter::Node<'t>> {
    let target_row = line.saturating_sub(1) as usize;
    let mut best: Option<tree_sitter::Node<'t>> = None;
    let mut cursor = root.walk();
    let mut stack = vec![*root];
    while let Some(node) = stack.pop() {
        if node.start_position().row == target_row
            && (node.kind() == "call_expression"
                || node.kind() == "call"
                || node.kind() == "macro_invocation")
        {
            best = Some(node);
        }
        for child in node.children(&mut cursor) {
            if child.start_position().row <= target_row
                && child.end_position().row >= target_row
            {
                stack.push(child);
            }
        }
    }
    best
}

/// Climb parents from `node` until we find an ancestor whose `kind()`
/// is in `stop_at`, or we've climbed `MAX_ANCESTOR_HOPS` levels.
fn climb_to_statement<'t>(
    node: tree_sitter::Node<'t>,
    stop_at: &[&str],
) -> Option<tree_sitter::Node<'t>> {
    let mut current = node;
    for _ in 0..MAX_ANCESTOR_HOPS {
        if stop_at.contains(&current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
    None
}

/// Return the source text of `node`, capped at `MAX_CONTEXT_LINES`.
fn extract_node_text(source: &str, node: tree_sitter::Node<'_>) -> Option<String> {
    let text = node.utf8_text(source.as_bytes()).ok()?;
    let lines: Vec<&str> = text.lines().collect();
    let capped = if lines.len() > MAX_CONTEXT_LINES {
        &lines[..MAX_CONTEXT_LINES]
    } else {
        &lines[..]
    };
    Some(capped.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write `source` to a tempfile with the given extension and
    /// return its path. Tempfile lives as long as the returned handle.
    fn tempfile_with_ext(source: &str, ext: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()
            .expect("tempfile");
        f.write_all(source.as_bytes()).expect("write");
        f
    }

    #[test]
    fn enclosing_stmt_rust_single_line_call() {
        let src = "fn main() {\n    let x = foo(1, 2);\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("let x = foo(1, 2)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_rust_multi_line_args() {
        let src = "fn main() {\n    let x = foo(\n        1,\n        2,\n    );\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        // The whole let-declaration (5 lines) should be returned.
        assert!(got.contains("let x = foo("), "got: {got:?}");
        assert!(got.contains("1,"), "got: {got:?}");
        assert!(got.contains("2,"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_python_assignment() {
        let src = "def main():\n    result = module.func(arg)\n    return result\n";
        let f = tempfile_with_ext(src, "py");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("result = module.func(arg)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_typescript_return_expr() {
        let src = "function main() {\n    return handle(req);\n}\n";
        let f = tempfile_with_ext(src, "ts");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("return handle(req)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_javascript_return_expr() {
        let src = "function main() {\n    return handle(req);\n}\n";
        let f = tempfile_with_ext(src, "js");
        let got = enclosing_statement(f.path(), 2).expect("some");
        assert!(got.contains("return handle(req)"), "got: {got:?}");
    }

    #[test]
    fn enclosing_stmt_none_on_unreadable_file() {
        let missing = std::path::Path::new("/tmp/does_not_exist_92837.rs");
        assert!(enclosing_statement(missing, 5).is_none());
    }

    #[test]
    fn enclosing_stmt_none_on_unsupported_language() {
        let src = "# just a markdown\n";
        let f = tempfile_with_ext(src, "md");
        assert!(enclosing_statement(f.path(), 1).is_none());
    }

    #[test]
    fn enclosing_stmt_fallback_on_ast_miss() {
        // A call inside an odd ancestor the AST heuristic may not
        // surface as a statement. The fallback line-window should
        // kick in and return a non-empty string anyway.
        let src = "fn main() {\n    match x { Some(y) => foo(y), _ => () };\n}\n";
        let f = tempfile_with_ext(src, "rs");
        let got = enclosing_statement(f.path(), 2).expect("some");
        // Either we got the full match expression via expression_statement
        // (if tree-sitter surfaces it that way), or we got the ±1 window.
        // Either way the result must contain the call line.
        assert!(got.contains("foo(y)"), "got: {got:?}");
    }
}
```

- [ ] **Step 3: Add `tempfile` as a dev-dependency if not already present**

Check:
```bash
grep -A2 "dev-dependencies" Cargo.toml | head -5
```

If `tempfile` isn't already listed, add it:
```bash
cargo add --dev tempfile
```

Per `CLAUDE.md`'s hard rules, never hand-edit Cargo.toml for deps — use `cargo add`.

- [ ] **Step 4: Run the new tests**

Run: `cargo test -p blastguard search::context_extract::tests -- --nocapture`
Expected: all 8 tests pass (or report any failures verbatim; common issues are grammar-crate version mismatches — if so, check `Cargo.toml` for the installed versions and match them in the parser setup).

- [ ] **Step 5: Clippy pedantic on the new file**

Run: `cargo clippy -p blastguard --all-targets -- -W clippy::pedantic -D warnings 2>&1 | grep -A2 context_extract`
Expected: zero warnings on the new file. If pedantic flags `must_use_candidate` on the public function, it already has `#[must_use]`; other nits fix in place.

- [ ] **Step 6: Commit**

```bash
git add src/search/mod.rs src/search/context_extract.rs Cargo.toml Cargo.lock
git commit -m "search/context_extract: AST-smart enclosing-statement extractor

New module encapsulates tree-sitter parsing + ancestor-climbing to
return the enclosing statement text around a call site at a given
line. Per-language thread-local parsers match src/parse/rust.rs's
pattern.

Fallback: ±1 line window when the AST heuristic misses (rare —
triggered when the call's ancestor chain doesn't hit a
language-appropriate statement kind within 8 hops).

Returns None only for unreadable files or unsupported languages.

8 unit tests across Rust / Python / TypeScript / JavaScript cover
single-line calls, multi-line arg lists, the fallback path, and the
None cases.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Extend `callers_of` to attach context when requested

**Files:**
- Modify: `src/search/structural.rs`

Adds `with_context: bool` as the fifth parameter and wires in `context_extract::enclosing_statement` for each hit when the flag is set.

- [ ] **Step 1: Write the failing tests**

In `src/search/structural.rs`, append inside `#[cfg(test)] mod tests { ... }`:

```rust
    #[test]
    fn callers_of_without_context_leaves_context_field_none() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let target = sym("target", "a.rs");
        let caller = sym("caller", "b.rs");
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });
        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), false);
        // Non-hint hit — should have signature but no context.
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert!(!real.is_empty());
        for h in real {
            assert!(h.context.is_none(), "expected no context, got: {:?}", h.context);
        }
    }

    #[test]
    fn callers_of_with_context_attaches_text() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        use std::io::Write;
        // Create a real file so context_extract has something to read.
        let tmpdir = tempfile::tempdir().expect("tmpdir");
        let caller_path = tmpdir.path().join("caller.rs");
        let caller_src = "fn caller() {\n    let _ = target(42, \"hello\");\n}\n";
        std::fs::File::create(&caller_path)
            .expect("create")
            .write_all(caller_src.as_bytes())
            .expect("write");

        let mut g = CodeGraph::new();
        let target = sym("target", tmpdir.path().join("a.rs").to_str().unwrap());
        let mut caller = sym("caller", caller_path.to_str().unwrap());
        caller.line_start = 1;
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 2, // call is on line 2
            confidence: Confidence::Certain,
        });
        let hits = callers_of(&g, "target", 10, tmpdir.path(), true);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert_eq!(real.len(), 1);
        let ctx = real[0].context.as_deref().expect("context attached");
        assert!(
            ctx.contains("target(42, \"hello\")"),
            "expected call literal in context, got: {ctx}"
        );
    }

    #[test]
    fn callers_of_respects_limit_with_context() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let target = sym("target", "a.rs");
        insert_with_centrality(&mut g, target.clone(), 0);
        // 15 callers; limit is 10.
        for i in 0..15u32 {
            let caller = sym(&format!("caller{i}"), "b.rs");
            insert_with_centrality(&mut g, caller.clone(), 0);
            g.insert_edge(Edge {
                from: caller.id.clone(),
                to: target.id.clone(),
                kind: EdgeKind::Calls,
                line: i + 1,
                confidence: Confidence::Certain,
            });
        }
        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), true);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert_eq!(real.len(), 10, "limit=10 must be respected even with context");
    }

    #[test]
    fn callers_of_with_context_degrades_gracefully_on_missing_file() {
        // Graph points at a file that doesn't exist on disk.
        // context_extract returns None; hit.context stays None; no panic.
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let target = sym("target", "does_not_exist_a.rs");
        let caller = sym("caller", "does_not_exist_b.rs");
        insert_with_centrality(&mut g, target.clone(), 0);
        insert_with_centrality(&mut g, caller.clone(), 0);
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });
        let hits = callers_of(&g, "target", 10, std::path::Path::new("."), true);
        let real: Vec<_> = hits.iter().filter(|h| !h.is_hint()).collect();
        assert!(!real.is_empty());
        // No panic. Context may be None or a fallback window — just check no crash.
        let _ = real[0].context.clone();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p blastguard search::structural::tests::callers_of_with_context`
Expected: compile failure — `callers_of` takes 4 args, tests pass 5.

- [ ] **Step 3: Update `callers_of` signature + logic**

Find the existing `pub fn callers_of(...)` in `src/search/structural.rs` (around line 59). Replace its signature and add context attachment at the end:

```rust
pub fn callers_of(
    graph: &CodeGraph,
    name: &str,
    max_hits: usize,
    project_root: &std::path::Path,
    with_context: bool,
) -> Vec<SearchHit> {
```

(existing body unchanged up to the point where `hits` is constructed)

After the block that builds the base `hits: Vec<SearchHit>` and BEFORE the importer-hint block appends (search for `// Cross-file importer hint` — insertion point is right before that comment), add:

```rust
    if with_context {
        for hit in &mut hits {
            if hit.is_hint() {
                continue;
            }
            hit.context =
                crate::search::context_extract::enclosing_statement(&hit.file, hit.line);
        }
    }
```

Note: `hit.context` field is added in Task 5. Until Task 5 lands, this line won't compile. That's the deliberate TDD order — Task 5 drives the field into existence.

- [ ] **Step 4: Run cargo check**

Run: `cargo check --all-targets 2>&1 | tail -10`
Expected: single error about `hit.context` field not existing on `SearchHit`. Next task fixes that.

- [ ] **Step 5: Commit (red state, intentional)**

```bash
git add src/search/structural.rs
git commit -m "search/structural: callers_of accepts with_context flag

Red-state commit: adds the fifth parameter and the per-hit
context_extract call. The hit.context field doesn't exist yet —
next task adds it.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Add `context` field to `SearchHit` + render with pipe prefix

**Files:**
- Modify: `src/search/hit.rs`

Final piece: the data field + renderer. After this lands the tree compiles.

- [ ] **Step 1: Write the failing tests**

Append to `#[cfg(test)] mod tests` in `src/search/hit.rs`:

```rust
    #[test]
    fn to_compact_line_with_context_adds_pipe_prefix() {
        let hit = SearchHit {
            file: PathBuf::from("/tmp/foo.rs"),
            line: 42,
            signature: Some("fn caller()".to_string()),
            snippet: None,
            context: Some("let x = target(1, 2);".to_string()),
        };
        let out = hit.to_compact_line(std::path::Path::new("/tmp"));
        assert!(out.contains("foo.rs:42 caller()"), "got: {out}");
        assert!(out.contains("  | let x = target(1, 2);"), "got: {out}");
    }

    #[test]
    fn to_compact_line_with_multi_line_context() {
        let hit = SearchHit {
            file: PathBuf::from("/tmp/foo.rs"),
            line: 10,
            signature: Some("fn caller()".to_string()),
            snippet: None,
            context: Some("let x = foo(\n    1,\n    2,\n);".to_string()),
        };
        let out = hit.to_compact_line(std::path::Path::new("/tmp"));
        // Every context line should be pipe-prefixed.
        let context_lines: Vec<_> = out.lines().filter(|l| l.starts_with("  | ")).collect();
        assert_eq!(context_lines.len(), 4, "got: {out}");
    }

    #[test]
    fn to_compact_line_without_context_unchanged() {
        let hit = SearchHit {
            file: PathBuf::from("/tmp/foo.rs"),
            line: 7,
            signature: Some("fn caller()".to_string()),
            snippet: None,
            context: None,
        };
        let out = hit.to_compact_line(std::path::Path::new("/tmp"));
        assert!(!out.contains("  | "), "no-context hit must not have pipe prefix, got: {out}");
    }
```

Note: these tests construct `SearchHit` with the new `context` field — until the field exists, they won't compile.

- [ ] **Step 2: Add the `context` field to `SearchHit`**

In `src/search/hit.rs`, find the `pub struct SearchHit` definition and add the field:

```rust
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchHit {
    pub file: PathBuf,
    pub line: u32,
    pub signature: Option<String>,
    pub snippet: Option<String>,
    /// Caller-site context (enclosing statement text) attached by
    /// `callers_of X with context`. Prefixed `  | ` per line when
    /// rendered via `to_compact_line`.
    pub context: Option<String>,
}
```

- [ ] **Step 3: Update the three existing constructors to include `context: None`**

Find `impl SearchHit { pub fn structural(...)`, `pub fn grep(...)`, and `pub fn empty_hint(...)` — each constructs a struct literal. Add `context: None` to each:

```rust
    #[must_use]
    pub fn structural(symbol: &Symbol) -> Self {
        Self {
            file: symbol.id.file.clone(),
            line: symbol.line_start,
            signature: Some(symbol.signature.clone()),
            snippet: None,
            context: None,
        }
    }

    #[must_use]
    pub fn grep(file: PathBuf, line: u32, snippet: String) -> Self {
        Self {
            file,
            line,
            signature: None,
            snippet: Some(snippet),
            context: None,
        }
    }

    #[must_use]
    pub fn empty_hint(message: &str) -> Self {
        Self {
            file: PathBuf::new(),
            line: 0,
            signature: Some(message.to_owned()),
            snippet: None,
            context: None,
        }
    }
```

- [ ] **Step 4: Update `to_compact_line` to render context**

Find `pub fn to_compact_line(&self, project_root: &std::path::Path) -> String` in `src/search/hit.rs`. Replace the closing block (the one that returns `format!("{path}:{} {body}", self.line)`) with:

```rust
        let head = if body.is_empty() {
            format!("{path}:{}", self.line)
        } else {
            format!("{path}:{} {body}", self.line)
        };
        match self.context.as_deref() {
            Some(ctx) if !ctx.is_empty() => {
                let indented = ctx
                    .lines()
                    .map(|l| format!("  | {l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("{head}\n{indented}")
            }
            _ => head,
        }
```

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: every test passes. The new tests in Task 3, Task 4, and Task 5 all green; no regressions.

- [ ] **Step 6: Commit**

```bash
git add src/search/hit.rs
git commit -m "search/hit: SearchHit carries optional context field

Adds SearchHit.context: Option<String> populated by
callers_of when with_context=true. to_compact_line renders each
context line with a \"  | \" prefix so agents see a visually-
distinct block after the signature. Existing no-context renders
are byte-for-byte identical.

All three existing constructors updated to initialise context=None.
3 new tests cover: single-line context render, multi-line context
render (each line pipe-prefixed), no-context render unchanged.

Closes the compile chain started in Tasks 1, 2, 4.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Advertise the new query in `BLASTGUARD_BIAS`

**Files:**
- Modify: `bench/prompts.py`

Single-line cheat-sheet edit so the agent knows to reach for the new query shape when the task asks about argument values or call details.

- [ ] **Step 1: Update the `callers of` bullet in `BLASTGUARD_BIAS`**

In `bench/prompts.py`, find the `"Who calls this function?"` bullet (~line 32-35). Replace with:

```python
- "Who calls this function?" →
  `blastguard_search '{"query":"callers of NAME"}'`. Returns structured
  caller list including cross-file callers (unambiguous-name targets
  only; ambiguous names fall back to a per-importer-file hint).
- "What does X get called WITH (argument values at call sites)?" →
  `blastguard_search '{"query":"callers of NAME with context"}'`.
  Same structured caller list, BUT each hit also includes the
  enclosing statement text around the call so you can see the
  actual argument literals without a follow-up read_file. Use this
  when the question is about call-site details (what values are
  passed), not just who calls.
```

- [ ] **Step 2: Commit**

```bash
git add bench/prompts.py
git commit -m "bench/prompts: advertise callers-with-context query variant

Lets the agent know when to reach for the new 'callers of X with
context' form — questions about argument values at call sites, which
the plain 'callers of X' answer can't surface.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: Full verification gate

**Files:** none modified.

- [ ] **Step 1: Compile check**

Run: `cargo check --all-targets`
Expected: zero warnings, zero errors.

- [ ] **Step 2: Test suite**

Run: `cargo test`
Expected: all tests pass. Count should be strictly greater than before by at least 18 (8 new in context_extract, 4 in structural, 3 in hit, 3 in query).

- [ ] **Step 3: Clippy pedantic**

Run: `cargo clippy --all-targets -- -W clippy::pedantic -D warnings`
Expected: zero warnings.

- [ ] **Step 4: Release build**

Run: `cargo build --release`
Expected: `Finished \`release\` profile`.

If any gate fails, stop and fix before moving to Task 8.

---

## Task 8: Live verification against the real BlastGuard repo

**Files:** none modified.

Confirms the feature works end-to-end on actual data, not just synthetic tests.

- [ ] **Step 1: Clear index + rebuild**

```bash
test -d .blastguard && rm -r .blastguard
cargo build --release 2>&1 | tail -3
```

- [ ] **Step 2: Query `callers of apply_edit` (baseline, no context)**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"manual","version":"1"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"callers of apply_edit"}}}
' | ./target/release/blastguard /home/adam/Documents/blastguard 2>/dev/null | tail -40
```

Record the output. Should show callers as `file:line:signature` with NO `  | ` context lines.

- [ ] **Step 3: Query `callers of apply_edit with context`**

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"manual","version":"1"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"callers of apply_edit with context"}}}
' | ./target/release/blastguard /home/adam/Documents/blastguard 2>/dev/null | tail -80
```

Record the output. Should show the same callers PLUS `  | ` context lines containing argument literals. At least one hit should contain a string literal (e.g. `"return 1"`, `"NOT_PRESENT"`, etc. from the test callers of `apply_edit`).

- [ ] **Step 4: Quality assertion**

If Step 3's output contains at least one string literal that a Round-13-style answer would struggle to hallucinate, the feature is working end-to-end. If not, the AST extraction is silently returning fallback windows on real code — investigate. Likely cause: tree-sitter grammar version mismatch or wrong statement kind list per language.

---

## Self-Review

Spec coverage against `docs/superpowers/specs/2026-04-24-callers-with-context-design.md`:

- **Query syntax** — Task 1 regex + classifier + tests.
- **AST-smart extraction** — Task 3 `context_extract` module.
- **Language-specific statement kinds** — Task 3 `RUST_STATEMENT_KINDS` / `PY_STATEMENT_KINDS` / `TS_STATEMENT_KINDS` (covers rs, py, ts, tsx, js).
- **Fallback on AST miss** — Task 3 `line_window_fallback` + `enclosing_stmt_fallback_on_ast_miss` test.
- **Caps** — Task 3 `MAX_CONTEXT_LINES=20` + Task 4 `callers_of_respects_limit_with_context` (per-hit 20 + 10 hits).
- **SearchHit field + rendering** — Task 5 `context: Option<String>` + `  | ` pipe-prefix rendering.
- **BLASTGUARD_BIAS update** — Task 6.
- **Verification gate** — Task 7.
- **Live verification** — Task 8.
- **Error handling** — Task 3 tests cover unreadable / unsupported-language / missing-ancestor paths; Task 4 covers missing-file-on-disk.

No placeholders. Types and signatures consistent across tasks: `context: Option<String>` in Task 5 matches the `hit.context = ...` in Task 4 matches the `enclosing_statement(...)` return in Task 3.

No gaps.
