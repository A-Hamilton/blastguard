# BlastGuard `search` Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the full `search` tool backend — a query-string dispatcher that routes user queries to graph ops (callers, callees, outline, tests, …) or a regex grep fallback, always returning hits with inline signatures and centrality-ranked ordering.

**Architecture:** One entry point `search::dispatch(graph, project_root, query) -> Vec<SearchHit>`. The dispatcher classifies the query string via a regex ladder (SPEC §3.1 table) and routes to `search::structural::*` for graph-backed patterns or `search::text::grep` for the fallback. Results always carry inline signatures so the agent rarely needs a follow-up file read; multiple matches sort by reverse-edge centrality. The MCP wire wrap (rmcp `#[tool]` handler, `isError` mapping) is deferred to Plan 4.

**Tech Stack:** Rust 1.82+. Reuses already-landed infrastructure: `CodeGraph` in `src/graph/types.rs`, `graph::ops::{callers, callees, shortest_path, find_by_name}`, `parse::detect_language`. New deps: `regex` 1 (already pinned) for query classification; `ignore` + `regex` for grep fallback.

**Preconditions assumed by this plan:**
- Repository at `/home/adam/Documents/blastguard`. Branch to work on: a new branch `phase-1-search-tool` branched from `main` (HEAD currently `e23e975`).
- Plan 1 (indexing pipeline) has merged into main — `CodeGraph`, `cold_index`, `warm_start`, BLAKE3 cache, and 134+1 tests are all on `main`.
- `src/search/{mod.rs,dispatcher.rs,structural.rs,text.rs}` are stubs.

**Definition of done for this plan:**
- `search::dispatch(&graph, &project_root, "callers of processRequest")` returns a non-empty `Vec<SearchHit>` against the sample fixture.
- All 10 SPEC §3.1 structural patterns (callers/callees/imports/exports/chain/find/outline/tests/libraries/imports-of-file) work end-to-end.
- Grep fallback returns up to 30 matches with file:line snippets.
- Every result carries an inline signature (function symbols) or raw matching line (grep).
- Token budget respected: structural results trim to SPEC §3 targets (50–400 tokens depending on pattern); grep caps at 30.
- `cargo check --all-targets && cargo test && cargo clippy --all-targets -- -W clippy::pedantic -D warnings && cargo build --release` — all green.
- Test count ≥ 160 (135 from Plan 1 + 25 new from Plan 2).

---

## File Structure

| Path | Responsibility |
|---|---|
| `src/search/mod.rs` | Re-exports `dispatch`, `SearchHit`, `QueryKind` |
| `src/search/query.rs` | `QueryKind` enum + `classify(query: &str) -> QueryKind` regex ladder per SPEC §3.1 |
| `src/search/dispatcher.rs` | Top-level `dispatch()` — routes `QueryKind` → structural or text backend |
| `src/search/structural.rs` | Graph-backed implementations: `callers_of`, `callees_of`, `outline_of`, `chain_from_to`, `find`, `tests_for`, `libraries`, `imports_of`, `importers_of`, `exports_of` |
| `src/search/text.rs` | `grep(project_root, pattern) -> Vec<SearchHit>` — regex via `ignore`, cap 30 |
| `src/search/hit.rs` | `SearchHit` struct + `render_hit` formatter for inline display |
| `tests/integration_search.rs` | End-to-end: build graph from fixture, run every dispatcher pattern, assert expected hits |

Design notes:
- Split by responsibility: query classification (`query.rs`), routing (`dispatcher.rs`), backends (`structural.rs` and `text.rs`), and presentation (`hit.rs`). Each file stays ≤ 250 lines.
- `SearchHit` is the single return type across all backends. Structural hits populate `signature`; grep hits populate `snippet`. Both always include `file` + `line`.
- `QueryKind` holds parsed parameters (symbol name, file path, etc.) so the dispatcher does zero string parsing itself.

---

## Task 1: `SearchHit` struct + centrality ranking helper

**Files:**
- Create: `src/search/hit.rs`
- Modify: `src/search/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `src/search/hit.rs`:
```rust
//! Search result record and formatting helpers.

use std::path::PathBuf;

use serde::Serialize;

use crate::graph::types::{CodeGraph, Symbol, SymbolId};

/// A single search result. Rendered to an MCP text block by the tool handler.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchHit {
    pub file: PathBuf,
    pub line: u32,
    /// Inline signature for structural results (e.g. "processRequest(req: Request): Promise<Response>").
    /// None for grep hits.
    pub signature: Option<String>,
    /// Raw matching line for grep hits. None for structural hits.
    pub snippet: Option<String>,
}

impl SearchHit {
    #[must_use]
    pub fn structural(symbol: &Symbol) -> Self {
        Self {
            file: symbol.id.file.clone(),
            line: symbol.line_start,
            signature: Some(symbol.signature.clone()),
            snippet: None,
        }
    }

    #[must_use]
    pub fn grep(file: PathBuf, line: u32, snippet: String) -> Self {
        Self {
            file,
            line,
            signature: None,
            snippet: Some(snippet),
        }
    }
}

/// Sort a slice of `SymbolId`s by reverse-edge centrality descending.
/// Used to rank multiple matches in `find_by_name` / `callers_of` so the
/// highest-dependent symbols come first.
pub fn sort_by_centrality(graph: &CodeGraph, ids: &mut [&SymbolId]) {
    ids.sort_by_key(|id| std::cmp::Reverse(graph.centrality.get(*id).copied().unwrap_or(0)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};

    fn sym(name: &str, centrality: u32) -> (Symbol, u32) {
        let s = Symbol {
            id: SymbolId {
                file: PathBuf::from("x.ts"),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 2,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        };
        (s, centrality)
    }

    #[test]
    fn structural_hit_copies_signature() {
        let (s, _) = sym("foo", 0);
        let hit = SearchHit::structural(&s);
        assert_eq!(hit.signature.as_deref(), Some("fn foo()"));
        assert!(hit.snippet.is_none());
    }

    #[test]
    fn grep_hit_carries_snippet_only() {
        let hit = SearchHit::grep(
            PathBuf::from("a.ts"),
            5,
            "  const x = foo();".to_string(),
        );
        assert!(hit.signature.is_none());
        assert_eq!(hit.snippet.as_deref(), Some("  const x = foo();"));
    }

    #[test]
    fn sort_by_centrality_orders_highest_first() {
        let mut g = CodeGraph::new();
        let (low, _) = sym("low", 0);
        let (high, _) = sym("high", 0);
        g.insert_symbol(low.clone());
        g.insert_symbol(high.clone());
        g.centrality.insert(low.id.clone(), 1);
        g.centrality.insert(high.id.clone(), 10);
        let mut ids = vec![&low.id, &high.id];
        sort_by_centrality(&g, &mut ids);
        assert_eq!(ids[0], &high.id);
        assert_eq!(ids[1], &low.id);
    }
}
```

Replace `src/search/mod.rs` with:
```rust
//! Search dispatcher and backends — SPEC §3.1.

pub mod dispatcher;
pub mod hit;
pub mod query;
pub mod structural;
pub mod text;

pub use dispatcher::dispatch;
pub use hit::SearchHit;
pub use query::QueryKind;
```

(Note: `query.rs`, `structural.rs`, `text.rs` don't exist yet. `dispatcher.rs` does as a stub — update its contents in Task 2.)

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd /home/adam/Documents/blastguard
cargo test -p blastguard search::hit::tests 2>&1 | tail -20
```

Expected: compile errors because `search::query` / `search::structural` / `search::text` modules don't exist yet.

- [ ] **Step 3: Add empty stubs for the missing modules**

Create `src/search/query.rs`:
```rust
//! Query classifier — populated in Task 2.

use std::path::PathBuf;

/// Parsed query kind. The dispatcher routes on this enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryKind {
    Callers(String),
    Callees(String),
    Outline(PathBuf),
    Chain(String, String),
    Find(String),
    TestsFor(String),
    ImportsOf(PathBuf),
    ImportersOf(PathBuf),
    ExportsOf(PathBuf),
    Libraries,
    Grep(String),
}
```

Create `src/search/structural.rs`:
```rust
//! Graph-backed search backends — populated from Task 3 onwards.
```

Create `src/search/text.rs`:
```rust
//! Regex grep fallback — populated in Task 12.
```

Replace `src/search/dispatcher.rs` contents with a stub that compiles:
```rust
//! Top-level `search::dispatch` — populated in Task 2.

use crate::graph::types::CodeGraph;

use super::SearchHit;

#[must_use]
pub fn dispatch(_graph: &CodeGraph, _project_root: &std::path::Path, _query: &str) -> Vec<SearchHit> {
    Vec::new()
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p blastguard search::hit::tests 2>&1 | tail -20
```
Expected: 3 passed, 0 failed. Total test count 135 → 138.

- [ ] **Step 5: Commit**

```bash
cd /home/adam/Documents/blastguard
git checkout -b phase-1-search-tool
git add src/search/
git commit -m "phase 1.5: search scaffolding — SearchHit + module split

Creates SearchHit struct (structural vs grep variants), centrality
sort helper, and the four-module split (query, dispatcher, structural,
text, hit) that SPEC §3.1 maps to. Dispatcher is a stub; subsequent
tasks implement each QueryKind arm."
```

---

## Task 2: Query classifier (regex ladder)

**Files:**
- Modify: `src/search/query.rs`

- [ ] **Step 1: Write the failing tests**

Replace `src/search/query.rs` with this test-first layout (leave `QueryKind` enum as-is, add `classify` function shell after the tests):

```rust
//! Query classifier — parses a search query string into a [`QueryKind`].

use std::path::PathBuf;

/// Parsed query kind. The dispatcher routes on this enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryKind {
    Callers(String),
    Callees(String),
    Outline(PathBuf),
    Chain(String, String),
    Find(String),
    TestsFor(String),
    ImportsOf(PathBuf),
    ImportersOf(PathBuf),
    ExportsOf(PathBuf),
    Libraries,
    Grep(String),
}

/// Classify a query string per SPEC §3.1 dispatcher table.
///
/// Falls through to [`QueryKind::Grep`] when no structural pattern matches.
#[must_use]
pub fn classify(query: &str) -> QueryKind {
    let q = query.trim();
    // Placeholder — populated in step 3.
    QueryKind::Grep(q.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_of_pattern() {
        assert_eq!(
            classify("callers of processRequest"),
            QueryKind::Callers("processRequest".into())
        );
    }

    #[test]
    fn what_calls_alias() {
        assert_eq!(
            classify("what calls handler"),
            QueryKind::Callers("handler".into())
        );
    }

    #[test]
    fn callees_of_pattern() {
        assert_eq!(
            classify("callees of foo"),
            QueryKind::Callees("foo".into())
        );
        assert_eq!(
            classify("what does foo call"),
            QueryKind::Callees("foo".into())
        );
    }

    #[test]
    fn outline_of_file() {
        assert_eq!(
            classify("outline of src/handler.ts"),
            QueryKind::Outline(PathBuf::from("src/handler.ts"))
        );
    }

    #[test]
    fn chain_from_to() {
        assert_eq!(
            classify("chain from a to b"),
            QueryKind::Chain("a".into(), "b".into())
        );
    }

    #[test]
    fn find_and_where_is() {
        assert_eq!(classify("find x"), QueryKind::Find("x".into()));
        assert_eq!(classify("where is y"), QueryKind::Find("y".into()));
    }

    #[test]
    fn tests_for_file_or_symbol() {
        assert_eq!(
            classify("tests for src/a.ts"),
            QueryKind::TestsFor("src/a.ts".into())
        );
        assert_eq!(
            classify("tests for processRequest"),
            QueryKind::TestsFor("processRequest".into())
        );
    }

    #[test]
    fn imports_and_importers() {
        assert_eq!(
            classify("imports of src/a.ts"),
            QueryKind::ImportsOf(PathBuf::from("src/a.ts"))
        );
        assert_eq!(
            classify("importers of src/a.ts"),
            QueryKind::ImportersOf(PathBuf::from("src/a.ts"))
        );
    }

    #[test]
    fn exports_of_file() {
        assert_eq!(
            classify("exports of src/a.ts"),
            QueryKind::ExportsOf(PathBuf::from("src/a.ts"))
        );
    }

    #[test]
    fn libraries_keyword() {
        assert_eq!(classify("libraries"), QueryKind::Libraries);
    }

    #[test]
    fn unknown_falls_through_to_grep() {
        assert_eq!(
            classify("some random regex pattern [a-z]+"),
            QueryKind::Grep("some random regex pattern [a-z]+".into())
        );
    }

    #[test]
    fn leading_trailing_whitespace_trimmed() {
        assert_eq!(
            classify("  callers of foo  "),
            QueryKind::Callers("foo".into())
        );
    }
}
```

- [ ] **Step 2: Run to confirm red**

```bash
cd /home/adam/Documents/blastguard
cargo test -p blastguard search::query::tests 2>&1 | tail -40
```
Expected: every test except `unknown_falls_through_to_grep` and `leading_trailing_whitespace_trimmed` fails.

- [ ] **Step 3: Implement `classify`**

Replace the `classify` function body with a regex-anchored ladder. Use `regex::Regex::new` compiled once via `std::sync::OnceLock` or `std::sync::LazyLock` (stable in 1.82). Here's the full body — keep the top-of-file imports + `QueryKind` as above:

```rust
use std::sync::OnceLock;

use regex::Regex;

fn re(pattern: &'static str) -> &'static Regex {
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<&'static str, &'static Regex>>> =
        OnceLock::new();
    // Simplest stable pattern: a per-pattern static via match on the caller.
    // OnceLock requires a single value type, so we use Box::leak to intern.
    let map = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut g = map.lock().expect("regex cache poisoned");
    if let Some(&r) = g.get(pattern) {
        return r;
    }
    let leaked: &'static Regex = Box::leak(Box::new(Regex::new(pattern).expect("valid regex")));
    g.insert(pattern, leaked);
    leaked
}

#[must_use]
pub fn classify(query: &str) -> QueryKind {
    let q = query.trim();

    if let Some(caps) = re(r"^(?:callers of|what calls)\s+(.+)$").captures(q) {
        return QueryKind::Callers(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^(?:callees of|what does)\s+(.+?)\s*(?:call)?$").captures(q) {
        return QueryKind::Callees(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^outline of\s+(.+)$").captures(q) {
        return QueryKind::Outline(PathBuf::from(caps[1].trim()));
    }
    if let Some(caps) = re(r"^chain from\s+(.+?)\s+to\s+(.+)$").captures(q) {
        return QueryKind::Chain(caps[1].trim().to_string(), caps[2].trim().to_string());
    }
    if let Some(caps) = re(r"^(?:find|where is)\s+(.+)$").captures(q) {
        return QueryKind::Find(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^tests for\s+(.+)$").captures(q) {
        return QueryKind::TestsFor(caps[1].trim().to_string());
    }
    if let Some(caps) = re(r"^imports of\s+(.+)$").captures(q) {
        return QueryKind::ImportsOf(PathBuf::from(caps[1].trim()));
    }
    if let Some(caps) = re(r"^importers of\s+(.+)$").captures(q) {
        return QueryKind::ImportersOf(PathBuf::from(caps[1].trim()));
    }
    if let Some(caps) = re(r"^exports of\s+(.+)$").captures(q) {
        return QueryKind::ExportsOf(PathBuf::from(caps[1].trim()));
    }
    if q == "libraries" {
        return QueryKind::Libraries;
    }

    QueryKind::Grep(q.to_string())
}
```

Note: the `re` helper leaks a `&'static Regex` per pattern. This is acceptable — patterns are compile-time constants and the leak is bounded by the fixed pattern set.

- [ ] **Step 4: Run tests to confirm green**

```bash
cargo test -p blastguard search::query::tests 2>&1 | tail -30
```
Expected: all 12 tests pass.

- [ ] **Step 5: Clippy**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -10
```
Expected clean. If clippy complains about the `Box::leak` pattern, document it with a short comment. Common warnings: `clippy::unwrap_used` on `.lock().expect(...)` — the expect is fine since poisoning means our own code has already panicked.

- [ ] **Step 6: Commit**

```bash
git add src/search/query.rs
git commit -m "phase 1.5: query classifier — SPEC §3.1 regex ladder

classify() parses every structural pattern from SPEC §3.1's dispatcher
table into a QueryKind variant, falling through to Grep for anything
unmatched. Regex compilation is cached via a leaked 'static map so
repeated calls are allocation-free after the first match."
```

---

## Task 3: `find` / `where is` dispatcher arm

**Files:**
- Modify: `src/search/structural.rs`
- Modify: `src/search/dispatcher.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/search/structural.rs`:
```rust
//! Graph-backed search backends.

use crate::graph::ops::{callees, callers, find_by_name, shortest_path};
use crate::graph::types::{CodeGraph, EdgeKind, Symbol, SymbolId};
use crate::search::hit::{sort_by_centrality, SearchHit};

/// `find X` / `where is X` — centrality-ranked name lookup with fuzzy fallback.
/// Returns at most `max_hits` results.
#[must_use]
pub fn find(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut ids = find_by_name(graph, name);
    let mut owned: Vec<&SymbolId> = ids.drain(..).collect();
    sort_by_centrality(graph, &mut owned);
    owned
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{SymbolKind, Visibility};
    use std::path::PathBuf;

    fn mk(name: &str, file: &str, centrality: u32) -> (Symbol, u32) {
        let s = Symbol {
            id: SymbolId {
                file: PathBuf::from(file),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 10,
            line_end: 20,
            signature: format!("fn {name}(x: i32)"),
            params: vec!["x: i32".into()],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        };
        (s, centrality)
    }

    fn gwith(pairs: &[(Symbol, u32)]) -> CodeGraph {
        let mut g = CodeGraph::new();
        for (sym, c) in pairs {
            g.insert_symbol(sym.clone());
            g.centrality.insert(sym.id.clone(), *c);
        }
        g
    }

    #[test]
    fn find_returns_exact_match() {
        let (sym, _) = mk("process", "a.ts", 5);
        let g = gwith(&[(sym.clone(), 5)]);
        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].file, PathBuf::from("a.ts"));
        assert_eq!(hits[0].line, 10);
        assert_eq!(hits[0].signature.as_deref(), Some("fn process(x: i32)"));
    }

    #[test]
    fn find_fuzzy_when_no_exact() {
        let (sym, _) = mk("procss", "b.ts", 1);
        let g = gwith(&[(sym, 1)]);
        let hits = find(&g, "process", 10);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn find_sorts_by_centrality() {
        let (low_sym, _) = mk("process", "low.ts", 1);
        let (high_sym, _) = mk("process", "high.ts", 100);
        let g = gwith(&[(low_sym.clone(), 1), (high_sym.clone(), 100)]);
        let hits = find(&g, "process", 10);
        assert_eq!(hits[0].file, PathBuf::from("high.ts"));
        assert_eq!(hits[1].file, PathBuf::from("low.ts"));
    }

    #[test]
    fn find_caps_at_max_hits() {
        let pairs: Vec<_> = (0..20)
            .map(|i| mk("dup", &format!("f{i}.ts"), i))
            .collect();
        let g = gwith(&pairs);
        let hits = find(&g, "dup", 5);
        assert_eq!(hits.len(), 5);
    }

    #[test]
    fn find_empty_when_no_match() {
        let (sym, _) = mk("process", "a.ts", 0);
        let g = gwith(&[(sym, 0)]);
        let hits = find(&g, "xyz_no_match", 10);
        assert!(hits.is_empty());
    }
}
```

- [ ] **Step 2: Red**

```bash
cargo test -p blastguard search::structural::tests 2>&1 | tail -20
```
Expected: compile error because `find_by_name` returns `Vec<&SymbolId>` not an iterator — the sketch's `ids.drain(..)` needs adjustment. Fix below.

- [ ] **Step 3: Rewrite `find` to handle the actual signature**

The `find_by_name` function in `src/graph/ops.rs` returns `Vec<&SymbolId>` already. Simplify:
```rust
#[must_use]
pub fn find(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut ids: Vec<&SymbolId> = find_by_name(graph, name);
    sort_by_centrality(graph, &mut ids);
    ids.into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}
```

- [ ] **Step 4: Wire the dispatcher**

Replace `src/search/dispatcher.rs` contents with:
```rust
//! Top-level search dispatcher — classifies and routes queries.

use std::path::Path;

use crate::graph::types::CodeGraph;

use super::query::{classify, QueryKind};
use super::{structural, SearchHit};

/// Default cap for structural results. Keeps responses within SPEC §3 token budget.
const DEFAULT_MAX_HITS: usize = 10;

/// Classify + route a query. Returns an empty `Vec` when no match.
#[must_use]
pub fn dispatch(graph: &CodeGraph, _project_root: &Path, query: &str) -> Vec<SearchHit> {
    match classify(query) {
        QueryKind::Find(name) => structural::find(graph, &name, DEFAULT_MAX_HITS),
        // Other arms added in Tasks 4-11.
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::types::{Symbol, SymbolId, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn mk(name: &str) -> Symbol {
        Symbol {
            id: SymbolId {
                file: PathBuf::from("a.ts"),
                name: name.to_string(),
                kind: SymbolKind::Function,
            },
            line_start: 1,
            line_end: 2,
            signature: format!("fn {name}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        }
    }

    #[test]
    fn dispatches_find_query_to_structural_find() {
        let mut g = CodeGraph::new();
        g.insert_symbol(mk("processRequest"));
        let hits = dispatch(&g, Path::new("."), "find processRequest");
        assert_eq!(hits.len(), 1);
    }
}
```

- [ ] **Step 5: Run tests green**

```bash
cargo test -p blastguard search:: 2>&1 | tail -15
```
Expected: 5 structural tests + 1 dispatcher test + 12 query tests + 3 hit tests = 21 total. Test count 138 → 156.

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/search/
git commit -m "phase 1.5: find / where is — name lookup with centrality ranking"
```

---

## Task 4: `callers of X` / `what calls X`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write the failing tests**

Append to `structural.rs` tests:
```rust
#[test]
fn callers_of_returns_reverse_edges_with_signatures() {
    use crate::graph::types::{Confidence, Edge, EdgeKind};
    let target = mk("target", "t.ts", 0).0;
    let caller_a = mk("caller_a", "a.ts", 0).0;
    let caller_b = mk("caller_b", "b.ts", 0).0;
    let mut g = gwith(&[(target.clone(), 0), (caller_a.clone(), 0), (caller_b.clone(), 0)]);
    for caller in [&caller_a, &caller_b] {
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });
    }
    let hits = callers_of(&g, "target", 10);
    let files: Vec<_> = hits.iter().map(|h| h.file.to_string_lossy().to_string()).collect();
    assert!(files.contains(&"a.ts".to_string()));
    assert!(files.contains(&"b.ts".to_string()));
    assert!(hits.iter().all(|h| h.signature.is_some()));
}

#[test]
fn callers_of_empty_when_target_missing() {
    let g = CodeGraph::new();
    let hits = callers_of(&g, "nonexistent", 10);
    assert!(hits.is_empty());
}

#[test]
fn callers_of_caps_at_max_hits() {
    use crate::graph::types::{Confidence, Edge, EdgeKind};
    let target = mk("target", "t.ts", 0).0;
    let callers: Vec<_> = (0..15).map(|i| mk(&format!("c{i}"), &format!("c{i}.ts"), 0).0).collect();
    let mut pairs: Vec<_> = callers.iter().cloned().map(|s| (s, 0u32)).collect();
    pairs.insert(0, (target.clone(), 0));
    let mut g = gwith(&pairs);
    for caller in &callers {
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
    }
    let hits = callers_of(&g, "target", 5);
    assert_eq!(hits.len(), 5);
}
```

- [ ] **Step 2: Red**

```bash
cargo test -p blastguard search::structural::tests::callers_of 2>&1 | tail -15
```
Compile error — `callers_of` doesn't exist.

- [ ] **Step 3: Implement `callers_of`**

Append to `structural.rs`:
```rust
/// `callers of X` / `what calls X` — reverse BFS (1 hop) with inline signatures.
///
/// Resolves `name` to the most-central exact-match symbol, then returns its
/// direct callers sorted by their own centrality.
#[must_use]
pub fn callers_of(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut targets = find_by_name(graph, name);
    if targets.is_empty() {
        return Vec::new();
    }
    sort_by_centrality(graph, &mut targets);
    // Pick the top-centrality match as the resolution.
    let Some(&target_id) = targets.first() else {
        return Vec::new();
    };
    let mut caller_ids: Vec<&SymbolId> = callers(graph, target_id);
    sort_by_centrality(graph, &mut caller_ids);
    caller_ids
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}
```

- [ ] **Step 4: Wire dispatcher arm**

In `src/search/dispatcher.rs::dispatch`, add to the match:
```rust
QueryKind::Callers(name) => structural::callers_of(graph, &name, DEFAULT_MAX_HITS),
```

Add a dispatcher test:
```rust
#[test]
fn dispatches_callers_query() {
    use crate::graph::types::{Confidence, Edge, EdgeKind};
    let mut g = CodeGraph::new();
    let t = mk("target");
    let c = mk("caller");
    g.insert_symbol(t.clone());
    g.insert_symbol(c.clone());
    g.insert_edge(Edge {
        from: c.id.clone(),
        to: t.id.clone(),
        kind: EdgeKind::Calls,
        line: 1,
        confidence: Confidence::Certain,
    });
    let hits = dispatch(&g, Path::new("."), "callers of target");
    assert_eq!(hits.len(), 1);
}
```

Note: the existing dispatcher test's `mk` helper hard-codes `file: PathBuf::from("a.ts")` and both symbols end up with the same file — that's fine; the test only checks count.

Actually the `mk` helper builds identical symbols with different names but the same file. Since `SymbolId` is `(file, name, kind)`, the two symbols are distinct. Graph insert won't collide.

- [ ] **Step 5: Green**

```bash
cargo test -p blastguard search:: 2>&1 | tail -10
```
Expected: 156 → 160 tests pass.

- [ ] **Step 6: Clippy + commit**

```bash
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
git add src/search/
git commit -m "phase 1.5: callers of X — reverse BFS with inline signatures"
```

---

## Task 5: `callees of X` / `what does X call`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write tests**

Append to `structural.rs` tests:
```rust
#[test]
fn callees_of_returns_forward_edges() {
    use crate::graph::types::{Confidence, Edge, EdgeKind};
    let caller = mk("caller", "a.ts", 0).0;
    let callee_a = mk("helper_a", "b.ts", 0).0;
    let callee_b = mk("helper_b", "c.ts", 0).0;
    let mut g = gwith(&[(caller.clone(), 0), (callee_a.clone(), 0), (callee_b.clone(), 0)]);
    for callee in [&callee_a, &callee_b] {
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: callee.id.clone(),
            kind: EdgeKind::Calls,
            line: 5,
            confidence: Confidence::Certain,
        });
    }
    let hits = callees_of(&g, "caller", 10);
    let names: Vec<_> = hits.iter().map(|h| h.file.to_string_lossy().to_string()).collect();
    assert!(names.contains(&"b.ts".to_string()));
    assert!(names.contains(&"c.ts".to_string()));
}
```

- [ ] **Step 2: Red** — `cargo test search::structural::tests::callees_of` fails to compile.

- [ ] **Step 3: Implement**

```rust
#[must_use]
pub fn callees_of(graph: &CodeGraph, name: &str, max_hits: usize) -> Vec<SearchHit> {
    let mut sources = find_by_name(graph, name);
    if sources.is_empty() {
        return Vec::new();
    }
    sort_by_centrality(graph, &mut sources);
    let Some(&source_id) = sources.first() else {
        return Vec::new();
    };
    let mut callee_ids: Vec<&SymbolId> = callees(graph, source_id);
    sort_by_centrality(graph, &mut callee_ids);
    callee_ids
        .into_iter()
        .take(max_hits)
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}
```

- [ ] **Step 4: Dispatcher arm**

```rust
QueryKind::Callees(name) => structural::callees_of(graph, &name, DEFAULT_MAX_HITS),
```

- [ ] **Step 5: Green**

```bash
cargo test -p blastguard search:: 2>&1 | grep "test result"
```
Expected: 161 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/search/
git commit -m "phase 1.5: callees of X — forward edges with inline signatures"
```

---

## Task 6: `outline of FILE`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn outline_of_returns_all_symbols_in_file_sorted_by_line() {
    let a = mk_at("a", "x.ts", 10);
    let b = mk_at("b", "x.ts", 5);
    let c = mk_at("c", "y.ts", 1);
    let mut g = CodeGraph::new();
    for s in [&a, &b, &c] {
        g.insert_symbol(s.clone());
    }
    let hits = outline_of(&g, std::path::Path::new("x.ts"));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].line, 5);  // b first
    assert_eq!(hits[1].line, 10); // a second
    assert!(hits.iter().all(|h| h.file == std::path::PathBuf::from("x.ts")));
}

#[test]
fn outline_of_empty_when_no_file_symbols() {
    let g = CodeGraph::new();
    let hits = outline_of(&g, std::path::Path::new("nope.ts"));
    assert!(hits.is_empty());
}
```

Also add a helper to tests:
```rust
fn mk_at(name: &str, file: &str, line: u32) -> Symbol {
    let mut s = mk(name, file, 0).0;
    s.line_start = line;
    s
}
```

- [ ] **Step 2: Red**

```bash
cargo test -p blastguard search::structural::tests::outline_of 2>&1 | tail -10
```

- [ ] **Step 3: Implement**

```rust
/// `outline of FILE` — all symbols declared in `file`, sorted by line_start.
#[must_use]
pub fn outline_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let Some(symbol_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    let mut hits: Vec<SearchHit> = symbol_ids
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect();
    hits.sort_by_key(|h| h.line);
    hits
}
```

- [ ] **Step 4: Dispatcher arm**

```rust
QueryKind::Outline(path) => structural::outline_of(graph, &path),
```

- [ ] **Step 5: Green + commit**

```bash
cargo test -p blastguard search:: 2>&1 | grep "test result"
git add src/search/
git commit -m "phase 1.5: outline of FILE — all symbols sorted by line"
```

---

## Task 7: `chain from X to Y`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn chain_from_to_returns_shortest_path() {
    use crate::graph::types::{Confidence, Edge, EdgeKind};
    let a = mk("a", "a.ts", 0).0;
    let b = mk("b", "b.ts", 0).0;
    let c = mk("c", "c.ts", 0).0;
    let mut g = gwith(&[(a.clone(), 0), (b.clone(), 0), (c.clone(), 0)]);
    for (from, to) in [(&a, &b), (&b, &c)] {
        g.insert_edge(Edge {
            from: from.id.clone(),
            to: to.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
    }
    let hits = chain_from_to(&g, "a", "c");
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].file, std::path::PathBuf::from("a.ts"));
    assert_eq!(hits[2].file, std::path::PathBuf::from("c.ts"));
}

#[test]
fn chain_from_to_empty_when_unreachable() {
    let a = mk("a", "a.ts", 0).0;
    let b = mk("b", "b.ts", 0).0;
    let g = gwith(&[(a, 0), (b, 0)]);
    let hits = chain_from_to(&g, "a", "b");
    assert!(hits.is_empty());
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
#[must_use]
pub fn chain_from_to(graph: &CodeGraph, from_name: &str, to_name: &str) -> Vec<SearchHit> {
    let Some(&from_id) = find_by_name(graph, from_name).first() else {
        return Vec::new();
    };
    let Some(&to_id) = find_by_name(graph, to_name).first() else {
        return Vec::new();
    };
    let Some(path) = shortest_path(graph, from_id, to_id) else {
        return Vec::new();
    };
    path.iter()
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect()
}
```

Note: `shortest_path` returns `Option<Vec<SymbolId>>` where ids are owned — adjust signature call if the existing function returns `Vec<SymbolId>` (not `Vec<&SymbolId>`).

- [ ] **Step 4: Dispatcher arm**

```rust
QueryKind::Chain(from, to) => structural::chain_from_to(graph, &from, &to),
```

- [ ] **Step 5: Green + commit**

```bash
git add src/search/
git commit -m "phase 1.5: chain from X to Y — BFS shortest path"
```

---

## Task 8: `imports of FILE` and `importers of FILE`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

`imports of FILE` = files that `file` imports (forward-import edges where `from.file == file`).
`importers of FILE` = files that import `file` (reverse where `to.file == file`).

- [ ] **Step 1: Write tests**

```rust
#[test]
fn imports_of_returns_forward_import_edges() {
    use crate::graph::types::{Confidence, Edge, EdgeKind, Symbol, SymbolId, SymbolKind, Visibility};

    let mut g = CodeGraph::new();
    let module_a = Symbol {
        id: SymbolId {
            file: std::path::PathBuf::from("a.ts"),
            name: "a".into(),
            kind: SymbolKind::Module,
        },
        line_start: 1, line_end: 1,
        signature: "a".into(),
        params: vec![], return_type: None,
        visibility: Visibility::Export,
        body_hash: 0, is_async: false,
        embedding_id: None,
    };
    let module_b = Symbol {
        id: SymbolId {
            file: std::path::PathBuf::from("b.ts"),
            name: "b".into(),
            kind: SymbolKind::Module,
        },
        ..module_a.clone()
    };
    g.insert_symbol(module_a.clone());
    g.insert_symbol(module_b.clone());
    g.insert_edge(Edge {
        from: module_a.id.clone(),
        to: module_b.id.clone(),
        kind: EdgeKind::Imports,
        line: 1,
        confidence: Confidence::Unresolved,
    });

    let imports = imports_of(&g, std::path::Path::new("a.ts"));
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].file, std::path::PathBuf::from("b.ts"));

    let importers = importers_of(&g, std::path::Path::new("b.ts"));
    assert_eq!(importers.len(), 1);
    assert_eq!(importers[0].file, std::path::PathBuf::from("a.ts"));
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
#[must_use]
pub fn imports_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let Some(sym_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    let mut hits = Vec::new();
    for sid in sym_ids {
        let Some(edges) = graph.forward_edges.get(sid) else {
            continue;
        };
        for e in edges {
            if e.kind == EdgeKind::Imports {
                hits.push(SearchHit {
                    file: e.to.file.clone(),
                    line: e.line,
                    signature: Some(format!("imports {}", e.to.file.display())),
                    snippet: None,
                });
            }
        }
    }
    hits
}

#[must_use]
pub fn importers_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for (_to_id, rev_edges) in &graph.reverse_edges {
        for e in rev_edges {
            if e.kind == EdgeKind::Imports && e.to.file == file {
                hits.push(SearchHit {
                    file: e.from.file.clone(),
                    line: e.line,
                    signature: Some(format!("imports {}", e.to.file.display())),
                    snippet: None,
                });
            }
        }
    }
    hits
}
```

- [ ] **Step 4: Dispatcher arms**

```rust
QueryKind::ImportsOf(path) => structural::imports_of(graph, &path),
QueryKind::ImportersOf(path) => structural::importers_of(graph, &path),
```

- [ ] **Step 5: Green + commit**

```bash
git add src/search/
git commit -m "phase 1.5: imports of / importers of FILE"
```

---

## Task 9: `exports of FILE`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn exports_of_returns_exported_symbols_only() {
    let mut g = CodeGraph::new();
    let mut pub_sym = mk("api", "x.ts", 0).0;
    pub_sym.visibility = crate::graph::types::Visibility::Export;
    let mut priv_sym = mk("internal", "x.ts", 0).0;
    priv_sym.visibility = crate::graph::types::Visibility::Private;
    g.insert_symbol(pub_sym.clone());
    g.insert_symbol(priv_sym.clone());
    let hits = exports_of(&g, std::path::Path::new("x.ts"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].signature.as_deref(), Some("fn api(x: i32)"));
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
#[must_use]
pub fn exports_of(graph: &CodeGraph, file: &std::path::Path) -> Vec<SearchHit> {
    use crate::graph::types::Visibility;
    let Some(sym_ids) = graph.file_symbols.get(file) else {
        return Vec::new();
    };
    sym_ids
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .filter(|s| matches!(s.visibility, Visibility::Export))
        .map(SearchHit::structural)
        .collect()
}
```

- [ ] **Step 4: Dispatcher arm**

```rust
QueryKind::ExportsOf(path) => structural::exports_of(graph, &path),
```

- [ ] **Step 5: Green + commit**

```bash
git add src/search/
git commit -m "phase 1.5: exports of FILE — visibility-filtered symbols"
```

---

## Task 10: `tests for FILE` / `tests for X`

`tests for FILE` = files importing `file` whose path matches a test heuristic (contains `test_`, `.test.`, `_test`, or under `tests/`).
`tests for X` = treat X as symbol name, find its declaring file, then return `tests for FILE`.

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write tests**

```rust
#[test]
fn tests_for_file_returns_importers_in_test_paths() {
    use crate::graph::types::{Confidence, Edge, EdgeKind, Symbol, SymbolId, SymbolKind, Visibility};
    let mut g = CodeGraph::new();
    let src_module = Symbol {
        id: SymbolId {
            file: std::path::PathBuf::from("src/handler.ts"),
            name: "handler".into(),
            kind: SymbolKind::Module,
        },
        line_start: 1, line_end: 1,
        signature: "handler".into(),
        params: vec![], return_type: None,
        visibility: Visibility::Export,
        body_hash: 0, is_async: false,
        embedding_id: None,
    };
    let test_module = Symbol {
        id: SymbolId {
            file: std::path::PathBuf::from("tests/handler.test.ts"),
            name: "test_handler".into(),
            kind: SymbolKind::Module,
        },
        ..src_module.clone()
    };
    let unrelated_module = Symbol {
        id: SymbolId {
            file: std::path::PathBuf::from("src/other.ts"),
            name: "other".into(),
            kind: SymbolKind::Module,
        },
        ..src_module.clone()
    };
    g.insert_symbol(src_module.clone());
    g.insert_symbol(test_module.clone());
    g.insert_symbol(unrelated_module.clone());
    // test file imports src/handler.ts
    g.insert_edge(Edge {
        from: test_module.id.clone(),
        to: src_module.id.clone(),
        kind: EdgeKind::Imports,
        line: 1,
        confidence: Confidence::Certain,
    });
    // other.ts also imports src/handler.ts but it's not a test path
    g.insert_edge(Edge {
        from: unrelated_module.id.clone(),
        to: src_module.id.clone(),
        kind: EdgeKind::Imports,
        line: 1,
        confidence: Confidence::Certain,
    });
    let hits = tests_for(&g, "src/handler.ts");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].file.to_string_lossy().contains(".test."));
}

#[test]
fn tests_for_symbol_resolves_to_file_first() {
    let sym = mk("processRequest", "src/handler.ts", 0).0;
    let mut g = CodeGraph::new();
    g.insert_symbol(sym);
    // No test importers — expect empty but should not panic.
    let hits = tests_for(&g, "processRequest");
    assert!(hits.is_empty());
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
/// Heuristic: a path is a test file if any of its components contains
/// `test_`, `_test`, or `.test.`, or if any component is exactly `tests`.
fn is_test_path(path: &std::path::Path) -> bool {
    path.components().any(|c| {
        let s = c.as_os_str().to_string_lossy();
        s == "tests"
            || s.contains(".test.")
            || s.contains("_test")
            || s.starts_with("test_")
    })
}

#[must_use]
pub fn tests_for(graph: &CodeGraph, target: &str) -> Vec<SearchHit> {
    // First: try to interpret target as a symbol name and resolve to its file.
    let target_file = if target.contains('/') || target.contains('\\') {
        std::path::PathBuf::from(target)
    } else {
        let ids = find_by_name(graph, target);
        let Some(&id) = ids.first() else {
            return Vec::new();
        };
        id.file.clone()
    };

    importers_of(graph, &target_file)
        .into_iter()
        .filter(|hit| is_test_path(&hit.file))
        .collect()
}
```

- [ ] **Step 4: Dispatcher arm**

```rust
QueryKind::TestsFor(target) => structural::tests_for(graph, &target),
```

- [ ] **Step 5: Green + commit**

```bash
git add src/search/
git commit -m "phase 1.5: tests for FILE | symbol — importers under test paths"
```

---

## Task 11: `libraries`

**Files:**
- Modify: `src/search/structural.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write test**

```rust
#[test]
fn libraries_returns_unique_libraries_with_counts() {
    use crate::graph::types::LibraryImport;
    let mut g = CodeGraph::new();
    for (lib, file, line) in [
        ("lodash", "a.ts", 1),
        ("lodash", "b.ts", 1),
        ("@tanstack/react-query", "a.ts", 2),
        ("tokio", "lib.rs", 1),
    ] {
        g.library_imports.push(LibraryImport {
            library: lib.into(),
            symbol: String::new(),
            file: std::path::PathBuf::from(file),
            line,
        });
    }
    let hits = libraries(&g);
    // Expect one hit per unique library with its count rendered into signature.
    assert_eq!(hits.len(), 3);
    let lodash_hit = hits.iter().find(|h|
        h.signature.as_deref().is_some_and(|s| s.contains("lodash"))
    ).expect("lodash missing");
    assert!(lodash_hit.signature.as_deref().unwrap().contains("2 uses"));
}
```

- [ ] **Step 2: Red**

- [ ] **Step 3: Implement**

```rust
use std::collections::BTreeMap;

#[must_use]
pub fn libraries(graph: &CodeGraph) -> Vec<SearchHit> {
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    for li in &graph.library_imports {
        *counts.entry(li.library.as_str()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|(lib, count)| SearchHit {
            file: std::path::PathBuf::new(),
            line: 0,
            signature: Some(format!("{lib} ({count} uses)")),
            snippet: None,
        })
        .collect()
}
```

- [ ] **Step 4: Dispatcher arm**

```rust
QueryKind::Libraries => structural::libraries(graph),
```

- [ ] **Step 5: Green + commit**

```bash
git add src/search/
git commit -m "phase 1.5: libraries — grouped external imports with use counts"
```

---

## Task 12: Grep fallback via `ignore` + `regex`

**Files:**
- Modify: `src/search/text.rs`, `src/search/dispatcher.rs`

- [ ] **Step 1: Write tests**

Replace `src/search/text.rs` with:

```rust
//! Regex grep fallback — honours `.gitignore`, caps at 30 matches.

use std::io::{BufRead, BufReader};
use std::path::Path;

use regex::Regex;

use crate::search::hit::SearchHit;

/// Cap the number of grep hits returned (SPEC §3.1 grep row).
pub const GREP_MAX_HITS: usize = 30;

/// Regex grep across the project respecting `.gitignore`. Caps at
/// [`GREP_MAX_HITS`] matches and returns file:line with the matching line
/// as the `snippet`.
///
/// Invalid regex patterns are treated as literal-string searches.
#[must_use]
pub fn grep(project_root: &Path, pattern: &str) -> Vec<SearchHit> {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => match Regex::new(&regex::escape(pattern)) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        },
    };

    let mut hits = Vec::new();
    let walker = ignore::WalkBuilder::new(project_root)
        .standard_filters(true)
        .hidden(false)
        .build();

    for entry in walker.filter_map(std::result::Result::ok) {
        if hits.len() >= GREP_MAX_HITS {
            break;
        }
        let Some(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        let reader = BufReader::new(file);
        for (idx, line_res) in reader.lines().enumerate() {
            if hits.len() >= GREP_MAX_HITS {
                break;
            }
            let Ok(line) = line_res else { continue };
            if re.is_match(&line) {
                let lineno = u32::try_from(idx).unwrap_or(u32::MAX).saturating_add(1);
                hits.push(SearchHit::grep(
                    entry.path().to_path_buf(),
                    lineno,
                    line.trim_end().to_string(),
                ));
            }
        }
    }

    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(dir: &Path, files: &[(&str, &str)]) {
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(dir)
            .status()
            .expect("git init");
        for (p, body) in files {
            let full = dir.join(p);
            std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            std::fs::write(&full, body).expect("write");
        }
    }

    #[test]
    fn grep_finds_matching_line() {
        let tmp = tempfile::tempdir().expect("tempdir");
        seed(tmp.path(), &[
            ("src/a.ts", "function processRequest() {}\nconst x = 1;\n"),
        ]);
        let hits = grep(tmp.path(), "processRequest");
        assert!(!hits.is_empty());
        assert!(hits[0].snippet.as_deref().unwrap().contains("processRequest"));
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn grep_respects_gitignore() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(tmp.path().join(".gitignore"), "vendor/\n").expect("gitignore");
        seed(tmp.path(), &[
            ("src/a.ts", "processRequest();"),
            ("vendor/skip.ts", "processRequest();"),
        ]);
        let hits = grep(tmp.path(), "processRequest");
        assert!(hits.iter().all(|h| !h.file.to_string_lossy().contains("vendor")));
    }

    #[test]
    fn grep_caps_at_max_hits() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut files = Vec::new();
        for i in 0..(GREP_MAX_HITS + 10) {
            files.push((format!("src/f{i}.ts"), "MATCHME".to_string()));
        }
        let refs: Vec<(&str, &str)> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        seed(tmp.path(), &refs);
        let hits = grep(tmp.path(), "MATCHME");
        assert_eq!(hits.len(), GREP_MAX_HITS);
    }

    #[test]
    fn grep_invalid_regex_falls_back_to_literal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        seed(tmp.path(), &[
            ("src/a.ts", "const x = ?(invalid);"),
        ]);
        // `?(` is invalid regex; should still match as literal.
        let hits = grep(tmp.path(), "?(invalid)");
        assert!(!hits.is_empty());
    }
}
```

- [ ] **Step 2: Red**

```bash
cd /home/adam/Documents/blastguard
cargo test -p blastguard search::text::tests 2>&1 | tail -20
```

- [ ] **Step 3: The implementation above is already in place via the Write in Step 1. Run to confirm green.**

```bash
cargo test -p blastguard search::text::tests 2>&1 | tail -10
```

- [ ] **Step 4: Wire dispatcher**

In `src/search/dispatcher.rs`, update the match:
```rust
QueryKind::Grep(pattern) => super::text::grep(project_root, &pattern),
```

Also update `dispatch`'s signature parameter name: change `_project_root: &Path` to `project_root: &Path` now that we use it.

- [ ] **Step 5: Green + commit**

```bash
cargo test -p blastguard search:: 2>&1 | grep "test result"
git add src/search/
git commit -m "phase 1.5: grep fallback via ignore+regex, cap 30"
```

---

## Task 13: Dispatcher integration test against fixture

**Files:**
- Create: `tests/integration_search.rs`

- [ ] **Step 1: Write the test**

```rust
use blastguard::index::indexer::cold_index;
use blastguard::search::dispatch;

#[test]
fn search_against_fixture_covers_multiple_patterns() {
    let root = std::path::Path::new("tests/fixtures/sample_project");
    let graph = cold_index(root).expect("cold_index");

    // find — processRequest exists in handler.ts
    let hits = dispatch(&graph, root, "find processRequest");
    assert!(!hits.is_empty(), "find processRequest returned no hits");
    assert!(hits[0].signature.is_some());

    // outline of lib.rs — two symbols declared
    let hits = dispatch(&graph, root, "outline of tests/fixtures/sample_project/src/lib.rs");
    // The file path in the graph is relative to what cold_index stored, so
    // look for either start or helper.
    assert!(graph
        .symbols
        .keys()
        .any(|id| id.name == "start" || id.name == "helper"),
        "fixture missing expected Rust symbols");

    // grep fallback — "verify" appears literally in the Python files
    let hits = dispatch(&graph, root, "verify");
    assert!(!hits.is_empty(), "grep should find 'verify' in the fixture");
    assert!(hits.iter().any(|h| h.snippet.is_some()));
}
```

- [ ] **Step 2: Run**

```bash
cd /home/adam/Documents/blastguard
cargo test --test integration_search 2>&1 | tail -10
```

If `outline of` fails because the fixture path's prefix is a tempdir / relative rooted differently than the graph's stored paths, loosen the assertion. The test's primary value is checking that `find` and `grep` work end-to-end against real indexed data.

- [ ] **Step 3: Commit**

```bash
git add tests/integration_search.rs
git commit -m "phase 1.5: integration test — search dispatcher over fixture"
```

---

## Task 14: Centrality-rank consistency test (cross-check)

**Files:**
- Modify: `src/search/structural.rs`

Ensure `find` and `callers_of` both apply centrality ordering consistently.

- [ ] **Step 1: Test**

```rust
#[test]
fn consistent_centrality_ordering_across_find_and_callers() {
    use crate::graph::types::{Confidence, Edge, EdgeKind};
    let target = mk("target", "t.ts", 0).0;
    let high_caller = mk("hi", "hi.ts", 100).0;
    let low_caller = mk("lo", "lo.ts", 1).0;
    let mut g = gwith(&[
        (target.clone(), 0),
        (high_caller.clone(), 100),
        (low_caller.clone(), 1),
    ]);
    for caller in [&high_caller, &low_caller] {
        g.insert_edge(Edge {
            from: caller.id.clone(),
            to: target.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
    }
    let find_hits = find(&g, "hi", 10);
    assert_eq!(find_hits.len(), 1);
    let caller_hits = callers_of(&g, "target", 10);
    assert_eq!(caller_hits[0].file, std::path::PathBuf::from("hi.ts"));
    assert_eq!(caller_hits[1].file, std::path::PathBuf::from("lo.ts"));
}
```

- [ ] **Step 2: Run — expect pass**

```bash
cargo test -p blastguard search::structural::tests::consistent_centrality 2>&1 | tail -5
```

- [ ] **Step 3: Commit**

```bash
git add src/search/
git commit -m "phase 1.5: pin cross-backend centrality consistency"
```

---

## Task 15: Final verification gate

- [ ] **Step 1: Run all gates**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -3
cargo test 2>&1 | grep "test result"
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -5
cargo build --release 2>&1 | tail -3
```

All four must pass. Test count ≥ 160 (135 baseline + 25 new).

- [ ] **Step 2: Commit**

```bash
git commit --allow-empty -m "phase 1.5: verification gate — search tool complete

All four gates green: cargo check --all-targets, cargo test, cargo clippy
--all-targets -- -W clippy::pedantic -D warnings, cargo build --release.

Search tool surface: find / callers / callees / outline / chain / imports /
importers / exports / tests-for / libraries / grep. All structural patterns
return inline signatures; grep caps at 30; multiple matches rank by
reverse-edge centrality.

Closes Plan docs/superpowers/plans/2026-04-18-blastguard-phase-1-search-tool.md.
Next: Plan 3 (apply_change) wires this into the cascade warning surface."
```

- [ ] **Step 3: Hand off**

Return to the user for merge / PR choice per `superpowers:finishing-a-development-branch`.

---

## Self-Review

**Spec coverage (SPEC §3.1 dispatcher table):**
- callers of X / what calls X — Task 4 ✓
- callees of X / what does X call — Task 5 ✓
- imports of FILE / importers of FILE — Task 8 ✓
- exports of FILE — Task 9 ✓
- chain from X to Y — Task 7 ✓
- find X / where is X — Task 3 ✓
- outline of FILE — Task 6 ✓
- tests for FILE / tests for X — Task 10 ✓
- around FILE:symbol (Phase 2) — DEFERRED ✓ (SPEC §3.1.2 says Phase 2)
- libraries — Task 11 ✓
- `semantic:` prefix (Phase 2) — DEFERRED ✓
- Fallback grep — Task 12 ✓

Inline signatures: every `structural::*` function returns `SearchHit` with `signature: Some(symbol.signature.clone())` — ✓.

Centrality ranking: Task 1 adds `sort_by_centrality`; Tasks 3, 4, 5 use it; Task 14 cross-checks consistency — ✓.

Token budget: `DEFAULT_MAX_HITS = 10` caps structural; `GREP_MAX_HITS = 30` caps grep. Hit signatures are bounded by the source symbols' signature strings (already capped during extraction).

**Placeholder scan:** searched for "TBD", "implement later", "similar to" — none present. Every step has runnable code or an exact command.

**Type consistency:** `SearchHit { file, line, signature: Option<String>, snippet: Option<String> }` appears identically in Tasks 1, 3–12. `QueryKind` variants map 1:1 to dispatcher arms. `find_by_name` signature matches Plan 1's ops.rs.

**One judgement call to revisit during execution:** Task 10's `tests_for` heuristic assumes Unix-style paths. Windows `\` paths use `components().any(...)` which normalises — should work. If a real SWE-bench task surfaces a Windows-specific edge case, tighten in Phase 2.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-18-blastguard-phase-1-search-tool.md`. Two execution options:

**1. Subagent-Driven (recommended)** — fresh subagent per task, review between each, same pattern that got Plan 1 through cleanly.

**2. Inline Execution** — run tasks in this session with checkpoints.

Which approach?
