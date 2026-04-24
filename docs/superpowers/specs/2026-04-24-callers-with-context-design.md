# `callers of NAME with context` — AST-smart call-site context

**Status:** Design, approved to proceed to plan.
**Date:** 2026-04-24.
**Scope:** Add a BlastGuard query variant that returns the enclosing statement around each caller's call site via tree-sitter, so an agent can see argument values without a follow-up `read_file`.

## Problem

Round 13's LLM-judge data showed BG loses the substance axis 20-7 to raw across 30 pairs. Investigation of representative losses surfaced three systematic gaps ([MICROBENCH.md Round 13](../../MICROBENCH.md)). The most common one is:

- **BG returns signatures; argument values live in the caller's body.**

Concrete example (Round 13, `callers-apply-edit` seed=3):

- Raw's answer listed exact argument literals at each call site: `"return 1"`, `"NOT_PRESENT"`, `"= 1"`, `"x"`, `"processRequest(req)"`, `"THIS_TEXT_DOES_NOT_EXIST"` — read directly from the test file bodies.
- BG's answer said `(likely passing specific text)`, `(likely passing an empty or non-matching string)` — vague guesses, because `callers_of` returns only `file:line:signature`, and the actual arguments passed at the call site live in the caller's function body, which BG never reads.

Today the agent can recover this by following `callers of X` with `read_file` on each hit, but that (a) requires the agent to know to do it, (b) spends tokens on full-file reads, and (c) loses the "one BG call gets the answer" cost-win.

## Goal

Close the most common substance gap at the tool layer. Success = on a representative "what does X get called with?" question, a single BlastGuard query returns enough context that the agent can answer specifically without a follow-up `read_file`.

## Non-goals

- **Not redesigning `callers of`.** The base query stays exactly as today — signature-only, cheap, default behavior unchanged.
- **Not addressing the other two gaps** (docstring mentions, integration tests via importers) — those get separate specs.
- **Not handling every call-site shape perfectly.** AST-smart covers the common cases; for edge cases (e.g. calls inside decorators, calls inside complex macro expansions) we fall back to a line window.

## Approach

Extend the existing `QueryKind::Callers` to carry an optional `with_context` flag. The classifier regex matches both forms. The dispatcher routes both to `structural::callers_of`; when the flag is set, each hit is augmented with a context block extracted via tree-sitter.

### Query syntax

```text
callers of NAME                    → signature-only (existing behavior, unchanged)
callers of NAME with context       → signature + enclosing statement per caller
```

Chosen because "with context" extends the existing vocabulary, is discoverable from the base query, and composes cleanly if we later add `with X` modifiers to other query types.

### What counts as "enclosing statement" (per language)

For each caller's `(file_path, line)` pair, the context extractor parses the file, finds the `call_expression` (or language equivalent) at the given line, and climbs parent nodes until it hits a statement-level boundary. The emitted text is the statement's byte range.

| language | enclosing-statement ancestors to stop at |
|---|---|
| Rust | `let_declaration`, `expression_statement`, `return_expression`, `macro_invocation`, `assignment_expression`, `call_expression` at statement position |
| TypeScript / JavaScript | `lexical_declaration`, `expression_statement`, `return_statement`, `assignment_expression` |
| Python | `expression_statement`, `assignment`, `return_statement`, `if_statement` (condition slot) |

If no match is found within 8 ancestor hops, fall back to `±1 line window` around the call. Falls back silently — never fails the query.

### Caps and budget

- Caller count cap: **10** (unchanged from existing `callers_of` behavior).
- Per-hit context size cap: **20 lines** (guards against pathological multi-line chains; extremely rare).
- Expected worst-case tokens: 10 callers × ~300 tokens/context = 3K tokens added to the response. Compared to raw's typical 15-50K on the same question, BG with context remains a 5-10× cost reduction — preserves the headline cost-win story.

## Components

### `src/search/query.rs`

Extend `QueryKind::Callers` from `(String)` to `(String, bool)` where the `bool` is `with_context`. Update the classifier regex:

```rust
// before
QueryKind::Callers(String),
// after
QueryKind::Callers(String, bool),  // second field: with_context

// regex
r"^(?:callers of|what calls)\s+(.+?)(\s+with\s+context)?$"
```

### `src/search/context_extract.rs` (new file)

Small, focused module with one public function:

```rust
/// Returns the text of the enclosing statement around the call
/// expression at `line` in `file`. Falls back to `±1` line window
/// when the enclosing-statement heuristic fails.
///
/// Best-effort: returns `None` only if the file is unreadable or
/// the language is unsupported.
#[must_use]
pub fn enclosing_statement(file: &Path, line: u32) -> Option<String>;
```

Internals:
1. Detect language from file extension.
2. Parse the file with the appropriate tree-sitter parser (already initialized per-thread per `CLAUDE.md`'s parser pattern).
3. Find the node at `line - 1` (0-indexed). Descend to the deepest `call_expression`-ish node at that line.
4. Climb parents until we hit a language-appropriate statement ancestor, or 8 hops deep, or no parent.
5. If found: return `node.utf8_text(source)` (capped at 20 lines).
6. If not found: return `±1 line window` from the source bytes.

### `src/search/structural.rs::callers_of`

Gains `with_context: bool` parameter. When `true`, for each hit calls `context_extract::enclosing_statement(&hit.file, hit.line)` and stores the result in `hit.context`.

### `src/search/hit.rs::SearchHit`

Gains an optional field:

```rust
pub struct SearchHit {
    pub file: PathBuf,
    pub line: u32,
    pub signature: Option<String>,
    pub snippet: Option<String>,
    pub context: Option<String>,  // NEW — caller's enclosing statement
}
```

Rendering: when `context` is set, the compact-line format becomes:

```text
src/edit/apply.rs:227 fn orchestrate(request: &ApplyChangeRequest, ...) -> Result<...>
  | for change in &request.changes {
  |     apply_edit(&file_path, &change.old_text, &change.new_text)?;
  | }
```

Each context line prefixed with `  | ` to visually distinguish from signature.

## Data flow

```
agent: blastguard_search '{"query":"callers of apply_edit with context"}'
  ↓
query::classify → QueryKind::Callers("apply_edit", with_context=true)
  ↓
dispatcher::dispatch → structural::callers_of(graph, "apply_edit", limit=10, with_context=true)
  ↓
for each hit:
  context_extract::enclosing_statement(hit.file, hit.line)
  → attach Some(text) or None to hit.context
  ↓
Vec<SearchHit> with context populated
  ↓
response rendered via hit.to_compact_line(project_root)
```

## Testing (TDD)

Red-phase tests (these fail before any implementation):

1. **Query classifier** (`src/search/query.rs`):
   - `callers of foo` → `QueryKind::Callers("foo".into(), false)` (current behavior preserved).
   - `callers of foo with context` → `QueryKind::Callers("foo".into(), true)`.
   - `what calls foo` → `QueryKind::Callers("foo".into(), false)` (alias still works).
   - `callers of foo with bar` → fallback: `Callers("foo with bar", false)` (lenient — if it doesn't match `with context` exactly, treat as a literal name; existing behavior for unknown suffixes).

2. **`context_extract::enclosing_statement`** (`src/search/context_extract.rs`):
   - `enclosing_stmt_rust_single_line_call` — file with `let x = foo(1, 2);`, line points to the call, returns `"let x = foo(1, 2);"`.
   - `enclosing_stmt_rust_multi_line_args` — file with call spanning 4 lines, returns all 4 lines.
   - `enclosing_stmt_python_assignment` — `result = module.func(arg)`, returns the whole assignment.
   - `enclosing_stmt_typescript_return_expr` — `return handle(req);`, returns the return statement.
   - `enclosing_stmt_fallback_on_unknown_structure` — file with call inside unusual ancestor (e.g. a match arm), falls back to ±1 window, returns non-empty.
   - `enclosing_stmt_none_on_unreadable_file` — missing file returns `None`, doesn't panic.
   - `enclosing_stmt_none_on_unsupported_language` — `.md` file returns `None`.

3. **`callers_of` with context** (`src/search/structural.rs`):
   - `callers_of_with_context_attaches_text` — graph with 2 Rust callers, `with_context=true`, verify `hit.context.is_some()` on each.
   - `callers_of_without_context_leaves_field_none` — same graph, `with_context=false`, verify `hit.context.is_none()`.
   - `callers_of_respects_limit_with_context` — graph with 15 callers, `limit=10`, verify exactly 10 hits returned.

4. **SearchHit rendering** (`src/search/hit.rs`):
   - `to_compact_line_with_context_formats_pipe_prefix` — hit with context, rendered output contains `"  | "` on each context line.
   - `to_compact_line_without_context_unchanged` — hit without context, rendered output matches today's format exactly.

5. **End-to-end integration** (if feasible as a Rust unit test; otherwise documented as a manual verification step):
   - Query `callers of apply_edit with context` against the real BlastGuard repo graph. Verify at least one hit's context contains a string literal (e.g. `"return 1"`) that would let an agent answer the round-13 question specifically.

## Verification before "done"

1. `cargo check --all-targets`
2. `cargo test` — all existing tests still pass; new tests added and passing
3. `cargo clippy --all-targets -- -W clippy::pedantic -D warnings` — zero warnings
4. `cargo build --release` — binary builds
5. **Live bench re-run**: `/bench-quick` against Qwen or Gemma with the updated release binary. BG's `callers-apply-edit` answer should now mention argument literals. Compare to Round 13 baseline.

## Risks & mitigations

- **Tree-sitter parse failure** on malformed files → `enclosing_statement` returns `None`, hit keeps signature-only shape, response degrades gracefully. No query failure.
- **Performance regression** from per-caller re-parse → mitigated by caller-count cap of 10 and per-hit 20-line context cap. Tree-sitter parsing of a ~1000-line file is ~5ms; 10 callers × 5ms = 50ms, well within the tool-call latency budget.
- **Context-line format conflicts** with existing compact-line renderer → mitigated by prefixing context lines with `  | ` so existing single-line parsing on the agent side is unaffected (still sees `file:line signature` as the first line of the hit).
- **Unsupported language** (not Rust / TS / JS / Python) → `enclosing_statement` returns `None`, fall back to signature-only.

## Out of scope (explicit deferrals)

- `callees of NAME with context` — same pattern applies but addresses a different question; separate follow-up.
- `find NAME with context` — find is a name-lookup, not a call-site query; context doesn't map.
- Integration tests via `importers_of` — separate gap (Round 13 `tests-for-apply-change`), separate spec.
- Docstring/comment text search — separate gap (Round 13 `impact-of-removing-libraries`), separate spec.

## Rollback

Revert the 4-file diff. The feature is additive behind the `with_context` flag; no existing behavior changes, so revert is mechanical.
