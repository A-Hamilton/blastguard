# `chain from X to FILE` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let BlastGuard's `chain from X to Y` accept a file path for `Y`, returning the BFS chain from `X` to the first symbol in that file plus all other candidate endpoints reached via direct Calls from the chain.

**Architecture:** Purely additive branch inside `src/search/structural.rs::chain_from_to`. A new predicate-terminated BFS helper `shortest_path_to_predicate` lands alongside the existing `shortest_path` in `src/graph/ops.rs`; the name-to-name function becomes a one-line wrapper. Path detection is a small conservative regex (contains `/` or `\`, or trailing segment ends in a known source extension). No change to the MCP tool schema, `SearchHit` type, dispatcher, or query classifier.

**Tech Stack:** Rust 1.79+, existing `tree-sitter`, `rmcp`, `tokio`, no new crates. Gemma 4 26B A4B Q4_K_M for verification via `bench/microbench.py`.

**Spec:** `docs/superpowers/specs/2026-04-20-chain-from-to-file-path-design.md`.

---

## File Structure

- **Modify `src/graph/ops.rs`** — add `shortest_path_to_predicate`; rewrite `shortest_path` as a thin wrapper. Add two unit tests for the predicate variant. (~40 lines added.)
- **Modify `src/search/structural.rs`** — add `is_path_like` helper, add `chain_to_file_path` internal routine, branch `chain_from_to` on path-likeness. Add four new unit tests. (~90 lines added.)
- **Modify `bench/prompts.py`** — one-line cheat-sheet edit under the `chain from A to B` bullet. (~2 lines changed.)
- **Modify `docs/MICROBENCH.md`** — append "Round 12" section with the measured outcome. (Post-verification.)

No new files. Existing patterns (colocated `#[cfg(test)] mod tests`, `thiserror` nowhere needed here since the function returns `Vec<SearchHit>` with empty-hint fallbacks).

---

## Task 1: Add predicate-terminated BFS helper

**Files:**
- Modify: `src/graph/ops.rs` (wrap existing `shortest_path` around a new helper)
- Test: `src/graph/ops.rs` (colocated `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests { ... }` in `src/graph/ops.rs` (before the closing brace):

```rust
    #[test]
    fn shortest_path_to_predicate_walks_forward_edges() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        let c = mk("c", "y.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        g.insert_symbol(c.clone());
        connect(&mut g, &a, &b);
        connect(&mut g, &b, &c);
        let path = shortest_path_to_predicate(&g, &a.id, |id| id.file == std::path::Path::new("y.ts"))
            .expect("reachable");
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], a.id);
        assert_eq!(path[2], c.id);
    }

    #[test]
    fn shortest_path_to_predicate_none_when_no_node_matches() {
        let mut g = CodeGraph::new();
        let a = mk("a", "x.ts");
        let b = mk("b", "x.ts");
        g.insert_symbol(a.clone());
        g.insert_symbol(b.clone());
        connect(&mut g, &a, &b);
        assert!(shortest_path_to_predicate(&g, &a.id, |_| false).is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p blastguard graph::ops::tests::shortest_path_to_predicate`
Expected: compile failure (`cannot find function 'shortest_path_to_predicate'`).

- [ ] **Step 3: Implement `shortest_path_to_predicate` and wrap `shortest_path`**

Replace the existing `shortest_path` function in `src/graph/ops.rs` (lines ~30-69) with:

```rust
/// BFS shortest path from `from` to any node matching `pred`, following
/// forward edges. `None` when unreachable. Returns the chain of symbols in
/// order, terminating at the first node for which `pred(id)` is true.
#[must_use]
pub fn shortest_path_to_predicate<F>(
    graph: &CodeGraph,
    from: &SymbolId,
    pred: F,
) -> Option<Vec<SymbolId>>
where
    F: Fn(&SymbolId) -> bool,
{
    if pred(from) {
        return Some(vec![from.clone()]);
    }
    let mut queue: VecDeque<SymbolId> = VecDeque::new();
    let mut visited: HashSet<SymbolId> = HashSet::new();
    let mut parent: std::collections::HashMap<SymbolId, SymbolId> =
        std::collections::HashMap::new();
    queue.push_back(from.clone());
    visited.insert(from.clone());

    while let Some(current) = queue.pop_front() {
        let Some(edges) = graph.forward_edges.get(&current) else {
            continue;
        };
        for edge in edges {
            if visited.insert(edge.to.clone()) {
                parent.insert(edge.to.clone(), current.clone());
                if pred(&edge.to) {
                    let mut path = vec![edge.to.clone()];
                    let mut node = edge.to.clone();
                    while let Some(p) = parent.get(&node) {
                        path.push(p.clone());
                        if p == from {
                            break;
                        }
                        node = p.clone();
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(edge.to.clone());
            }
        }
    }
    None
}

/// BFS shortest path from `from` to `to`, following forward edges. `None`
/// when unreachable. Returns the chain of symbols in order.
#[must_use]
pub fn shortest_path(graph: &CodeGraph, from: &SymbolId, to: &SymbolId) -> Option<Vec<SymbolId>> {
    shortest_path_to_predicate(graph, from, |id| id == to)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p blastguard graph::ops::tests`
Expected: all four graph::ops tests pass (existing two + two new).

- [ ] **Step 5: Commit**

```bash
git add src/graph/ops.rs
git commit -m "graph/ops: add shortest_path_to_predicate BFS helper

Predicate-terminated BFS that generalises the existing name-to-name
shortest_path (now a one-line wrapper). Unblocks the file-path endpoint
in \`chain from X to Y\`.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Add `is_path_like` helper and failing path-mode tests

**Files:**
- Modify: `src/search/structural.rs` (add free fn + four tests)
- Test: `src/search/structural.rs` (colocated `#[cfg(test)] mod tests`)

- [ ] **Step 1: Add the `is_path_like` helper**

Insert near the other private helpers in `src/search/structural.rs` (right above `module_source_id`, around line 196, before the `/// Canonical SymbolId for the synthetic "import source"...` docstring):

```rust
/// Heuristic: does `s` look like a file path rather than a symbol name?
/// True when `s` contains `/` or `\`, or when its trailing segment ends in
/// a known source extension. Deliberately conservative so bare identifiers
/// and qualified names like `module::fn` never trip this.
fn is_path_like(s: &str) -> bool {
    if s.contains('/') || s.contains('\\') {
        return true;
    }
    let lower = s.to_ascii_lowercase();
    const EXTS: &[&str] = &[".rs", ".ts", ".tsx", ".js", ".jsx", ".py"];
    EXTS.iter().any(|ext| lower.ends_with(ext))
}
```

- [ ] **Step 2: Write the failing path-mode tests**

Append inside `mod tests { ... }` at the end of `src/search/structural.rs`, just before the closing `}` of the module (search for the last `#[test]` near the end of the file and add these after it):

```rust
    #[test]
    fn chain_to_file_path_returns_chain_plus_file_candidates() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        // search_tool (mcp/server.rs) -> dispatch (search/dispatcher.rs)
        //   -> find (search/structural.rs)
        //   dispatch also calls callers_of in structural.rs directly.
        let search_tool = sym("search_tool", "src/mcp/server.rs");
        let dispatch = sym("dispatch", "src/search/dispatcher.rs");
        let find = sym("find", "src/search/structural.rs");
        let callers = sym("callers_of", "src/search/structural.rs");
        insert_with_centrality(&mut g, search_tool.clone(), 0);
        insert_with_centrality(&mut g, dispatch.clone(), 0);
        insert_with_centrality(&mut g, find.clone(), 5);
        insert_with_centrality(&mut g, callers.clone(), 2);
        for (from, to) in [
            (&search_tool, &dispatch),
            (&dispatch, &find),
            (&dispatch, &callers),
        ] {
            g.insert_edge(Edge {
                from: from.id.clone(),
                to: to.id.clone(),
                kind: EdgeKind::Calls,
                line: 1,
                confidence: Confidence::Certain,
            });
        }
        let hits = chain_from_to(&g, "search_tool", "src/search/structural.rs");
        // First three hits are the chain search_tool -> dispatch -> find.
        assert!(hits.len() >= 3, "expected chain + candidates, got {hits:?}");
        assert_eq!(hits[0].file, PathBuf::from("src/mcp/server.rs"));
        assert_eq!(hits[1].file, PathBuf::from("src/search/dispatcher.rs"));
        assert_eq!(hits[2].file, PathBuf::from("src/search/structural.rs"));
        // `callers_of` is a sibling candidate reached from a chain node.
        let candidate_names: Vec<&str> = hits
            .iter()
            .skip(3)
            .filter_map(|h| h.signature.as_deref())
            .collect();
        assert!(
            candidate_names.iter().any(|s| s.contains("callers_of")),
            "expected callers_of in candidates, got {candidate_names:?}"
        );
    }

    #[test]
    fn chain_to_file_path_backward_compat_with_symbol_name() {
        // A bare identifier must still route through the existing
        // name-to-name BFS — this mirrors chain_from_to_returns_shortest_path
        // and guards against the path-like heuristic overreaching.
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let a = sym("a", "a.ts");
        let b = sym("b", "b.ts");
        insert_with_centrality(&mut g, a.clone(), 0);
        insert_with_centrality(&mut g, b.clone(), 0);
        g.insert_edge(Edge {
            from: a.id.clone(),
            to: b.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = chain_from_to(&g, "a", "b");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].file, PathBuf::from("a.ts"));
        assert_eq!(hits[1].file, PathBuf::from("b.ts"));
    }

    #[test]
    fn chain_to_file_path_unreachable_falls_back_with_hint() {
        let mut g = CodeGraph::new();
        // `from` exists, target file has an indexed symbol, but no call
        // edges connect them.
        insert_with_centrality(&mut g, sym("caller", "src/a.rs"), 0);
        insert_with_centrality(&mut g, sym("island", "src/unreachable.rs"), 0);
        let hits = chain_from_to(&g, "caller", "src/unreachable.rs");
        assert!(
            hits.iter().any(|h| h.file == PathBuf::from("src/a.rs")),
            "expected `from` hit, got {hits:?}"
        );
        assert!(
            hits.iter().any(|h| h
                .signature
                .as_deref()
                .is_some_and(|s| s.contains("no call-graph path"))),
            "expected unreachable hint, got {hits:?}"
        );
    }

    #[test]
    fn chain_to_file_path_when_from_already_in_target_file() {
        use crate::graph::types::{Confidence, Edge, EdgeKind};
        let mut g = CodeGraph::new();
        let a = sym("a", "src/x.rs");
        let b = sym("b", "src/x.rs");
        insert_with_centrality(&mut g, a.clone(), 0);
        insert_with_centrality(&mut g, b.clone(), 0);
        g.insert_edge(Edge {
            from: a.id.clone(),
            to: b.id.clone(),
            kind: EdgeKind::Calls,
            line: 1,
            confidence: Confidence::Certain,
        });
        let hits = chain_from_to(&g, "a", "src/x.rs");
        // Chain is just [a] (already in target), candidates include b.
        assert!(hits.iter().any(|h| h.signature.as_deref() == Some("fn a(x: i32)")));
        assert!(hits.iter().any(|h| h.signature.as_deref() == Some("fn b(x: i32)")));
    }

    #[test]
    fn chain_to_file_path_when_file_not_indexed() {
        let mut g = CodeGraph::new();
        insert_with_centrality(&mut g, sym("caller", "src/a.rs"), 0);
        let hits = chain_from_to(&g, "caller", "src/does_not_exist.rs");
        assert_eq!(hits.len(), 1);
        assert!(hits[0].is_hint());
        assert!(hits[0]
            .signature
            .as_deref()
            .is_some_and(|s| s.contains("no symbols indexed")));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p blastguard search::structural::tests::chain_to_file_path`
Expected: 4 tests fail. `chain_to_file_path_returns_chain_plus_file_candidates` fails with an assertion (path-like detection not wired into `chain_from_to` yet, so it falls through to the missing-symbol hint branch). `chain_to_file_path_backward_compat_with_symbol_name` passes because that behavior is unchanged.

- [ ] **Step 4: Commit failing tests + helper**

```bash
git add src/search/structural.rs
git commit -m "structural: add is_path_like helper + failing path-mode tests

Red-phase commit: new tests for file-path endpoint in chain_from_to
plus the is_path_like heuristic they will use. Green implementation
lands next.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Implement the path-mode branch in `chain_from_to`

**Files:**
- Modify: `src/search/structural.rs` — extend `chain_from_to` to branch on `is_path_like`.

- [ ] **Step 1: Replace `chain_from_to` with the two-mode version**

Locate the current `chain_from_to` at `src/search/structural.rs:159` and replace it (and the preceding docstring) with:

```rust
/// `chain from X to Y` — BFS shortest path across forward call edges.
///
/// Two modes:
///
/// - **Symbol mode** (`Y` is a bare name): returns the shortest `Vec` of
///   structural hits from the `from` symbol to the `to` symbol. If either
///   name doesn't resolve, or no path exists, returns a hint-shaped hit
///   with guidance.
/// - **Path mode** (`Y` is a file path — contains `/`, `\`, or ends in a
///   known source extension): returns the shortest chain from `from` into
///   any symbol whose file matches `Y`, followed by structural hits for
///   every other symbol in the target file that is a direct Calls-successor
///   of any node on the chain. Agents use this to answer "which function
///   in FILE does X reach?" in a single query.
///
/// Empty `Vec` is reserved for "neither endpoint exists" so the dispatcher
/// can distinguish the cases.
#[must_use]
pub fn chain_from_to(graph: &CodeGraph, from_name: &str, to_name: &str) -> Vec<SearchHit> {
    let from_ids = find_by_name(graph, from_name);
    let Some(&from_id) = from_ids.first() else {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{from_name}' found; try `find {from_name}` for fuzzy matches"
        ))];
    };

    if is_path_like(to_name) {
        return chain_to_file_path(graph, from_id, to_name);
    }

    let to_ids = find_by_name(graph, to_name);
    let Some(&to_id) = to_ids.first() else {
        return vec![SearchHit::empty_hint(&format!(
            "no symbol named '{to_name}' found; try `find {to_name}` for fuzzy matches"
        ))];
    };
    if let Some(path) = crate::graph::ops::shortest_path(graph, from_id, to_id) {
        return path
            .iter()
            .filter_map(|id| graph.symbols.get(id))
            .map(SearchHit::structural)
            .collect();
    }

    // No graph path — return the two endpoints plus a hint so the agent
    // has somewhere to go next instead of a dead-end empty response.
    let mut hits: Vec<SearchHit> = Vec::new();
    if let Some(sym) = graph.symbols.get(from_id) {
        hits.push(SearchHit::structural(sym));
    }
    if let Some(sym) = graph.symbols.get(to_id) {
        hits.push(SearchHit::structural(sym));
    }
    hits.push(SearchHit::empty_hint(&format!(
        "no call-graph path from {from_name} to {to_name} — Phase 1 doesn't follow re-export chains (`pub use`) or dynamic dispatch. \
         Try `imports of <from-file>` and `callers of {to_name}` to bridge manually, or grep for intermediate call sites."
    )));
    hits
}

/// Path-mode of `chain_from_to`: BFS from `from_id` to the first symbol in
/// the target file, plus sibling Calls-successors in the same file.
///
/// Caps the candidate list at [`CHAIN_FILE_CANDIDATE_CAP`] to keep the
/// response bounded on large target files.
fn chain_to_file_path(graph: &CodeGraph, from_id: &SymbolId, to_path: &str) -> Vec<SearchHit> {
    use std::path::Path;

    let target = Path::new(to_path);

    // File not indexed at all → useful hint so the agent doesn't retry blindly.
    let any_symbol_in_file = graph
        .symbols
        .keys()
        .any(|id| id.file.ends_with(target));
    if !any_symbol_in_file {
        return vec![SearchHit::empty_hint(&format!(
            "no symbols indexed in {to_path}; try `outline of {to_path}` or check the path spelling"
        ))];
    }

    let Some(path) = crate::graph::ops::shortest_path_to_predicate(graph, from_id, |id| {
        id.file.ends_with(target)
    }) else {
        // Indexed but unreachable via forward call edges.
        let mut hits: Vec<SearchHit> = Vec::new();
        if let Some(sym) = graph.symbols.get(from_id) {
            hits.push(SearchHit::structural(sym));
        }
        hits.push(SearchHit::empty_hint(&format!(
            "no call-graph path from {from_name} to any symbol in {to_path}; try `callees of {from_name}` then filter by file",
            from_name = from_id.name,
        )));
        return hits;
    };

    let mut hits: Vec<SearchHit> = path
        .iter()
        .filter_map(|id| graph.symbols.get(id))
        .map(SearchHit::structural)
        .collect();

    // Sibling candidates: symbols in the target file that are direct
    // Calls-successors of any node on the chain, excluding chain nodes.
    let chain_ids: std::collections::HashSet<&SymbolId> = path.iter().collect();
    let mut seen: std::collections::HashSet<&SymbolId> = std::collections::HashSet::new();
    let mut candidates: Vec<&Symbol> = Vec::new();
    for node in &path {
        let Some(edges) = graph.forward_edges.get(node) else {
            continue;
        };
        for e in edges {
            if e.kind != EdgeKind::Calls {
                continue;
            }
            if !e.to.file.ends_with(target) {
                continue;
            }
            if chain_ids.contains(&e.to) {
                continue;
            }
            if !seen.insert(&e.to) {
                continue;
            }
            if let Some(sym) = graph.symbols.get(&e.to) {
                candidates.push(sym);
            }
        }
    }
    // Centrality-sorted, highest first.
    candidates.sort_by_key(|s| {
        std::cmp::Reverse(graph.centrality.get(&s.id).copied().unwrap_or(0))
    });
    let overflow = candidates.len().saturating_sub(CHAIN_FILE_CANDIDATE_CAP);
    for sym in candidates.into_iter().take(CHAIN_FILE_CANDIDATE_CAP) {
        hits.push(SearchHit::structural(sym));
    }
    if overflow > 0 {
        hits.push(SearchHit::empty_hint(&format!(
            "{overflow} more candidate symbols in {to_path} truncated; use `outline of {to_path}` for the full list"
        )));
    }

    hits
}

/// Upper bound on the candidate-endpoint list in `chain_to_file_path`
/// responses. Matches the per-query cap convention used elsewhere.
const CHAIN_FILE_CANDIDATE_CAP: usize = 10;
```

Note: `Symbol` is already imported at the top of the file. If the compiler reports an unused import or missing import, fix it locally — do not introduce unrelated edits.

- [ ] **Step 2: Run the new tests to verify they pass**

Run: `cargo test -p blastguard search::structural::tests::chain_to_file_path`
Expected: all 5 path-mode tests pass (4 new + the backward-compat one).

- [ ] **Step 3: Run the full structural test module to confirm no regressions**

Run: `cargo test -p blastguard search::structural::tests`
Expected: every test in `structural.rs` passes, including the three existing `chain_from_to_*` tests.

- [ ] **Step 4: Run the complete test suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 5: Clippy check**

Run: `cargo clippy --all-targets -- -W clippy::pedantic -D warnings`
Expected: zero warnings. If pedantic flags a nit in the new code (e.g. `must_use_candidate` on `is_path_like`), fix it in place — add `#[must_use]` where the suggestion applies, or `#[allow(clippy::..)]` with a one-line reason comment only when the lint is wrong for the case.

- [ ] **Step 6: Commit**

```bash
git add src/search/structural.rs
git commit -m "structural: chain_from_to accepts file-path endpoint

When Y is a file path (contains / or \\ or ends in .rs/.ts/.tsx/.js/.jsx/.py),
BFS walks forward Calls edges until reaching a symbol in that file, then
appends direct sibling callees in the same file (centrality-sorted, capped
at 10). Closes the bench round 9-11 chain-search-to-graph regression where
the agent knew the target module but not a specific function in it.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Advertise the file-path form in `BLASTGUARD_BIAS`

**Files:**
- Modify: `bench/prompts.py` (one bullet)

- [ ] **Step 1: Update the cheat-sheet bullet**

In `bench/prompts.py`, locate the `chain from A to B` bullet (around line 42). Replace:

```python
- "What's the call chain from A to B?" →
  `blastguard_search '{"query":"chain from A to B"}'`. Returns the
  shortest path through the call graph when one exists, or both
  endpoints + a hint when re-exports block the direct path. Use this
  INSTEAD of stitching together multiple `callers of` / `callees of`
  queries when the question is "how does X reach Y".
```

with:

```python
- "What's the call chain from A to B?" →
  `blastguard_search '{"query":"chain from A to B"}'`. Returns the
  shortest path through the call graph when one exists, or both
  endpoints + a hint when re-exports block the direct path. `B` may
  also be a file path (e.g. `src/search/structural.rs`) — BlastGuard
  walks to the first symbol in that file and includes sibling callees
  in one response, so you don't need to guess the exact function name
  when you know the target module. Use this INSTEAD of stitching
  together multiple `callers of` / `callees of` queries when the
  question is "how does X reach Y".
```

- [ ] **Step 2: Commit**

```bash
git add bench/prompts.py
git commit -m "bench/prompts: advertise file-path endpoint for chain search

Lets the agent query 'chain from search_tool to src/search/structural.rs'
when it knows the target module but not a specific function. Matches
the new structural.rs path-mode branch landed in the previous commit.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Full verification gate

**Files:** none modified.

- [ ] **Step 1: Compile check**

Run: `cargo check --all-targets`
Expected: zero warnings, zero errors.

- [ ] **Step 2: Test suite**

Run: `cargo test`
Expected: all tests pass. Note the count — should be strictly greater than the pre-change count by at least 6 (2 in graph/ops + 4-5 in structural).

- [ ] **Step 3: Clippy pedantic**

Run: `cargo clippy --all-targets -- -W clippy::pedantic -D warnings`
Expected: zero warnings.

- [ ] **Step 4: Release build**

Run: `cargo build --release`
Expected: `Finished \`release\` profile`.

If any gate fails, **stop and fix the underlying issue before moving to Task 6**. Do not proceed to the bench re-run until all four gates are clean.

---

## Task 6: Round-12 bench re-run and write-up

**Files:**
- Create: `bench/runs/<date>-round12-chain-to-file.jsonl` (+ `.judge.jsonl`)
- Modify: `docs/MICROBENCH.md` (append Round 12 section)

- [ ] **Step 1: Run the bench-rerun skill's setup ritual**

Follow `.claude/skills/bench-rerun/SKILL.md` steps 1-8 strictly. The key steps that must not be skipped:

```bash
# (1) Liveness check
curl -m 30 -H "Content-Type: application/json" -H "Authorization: Bearer sk-local" \
  -d '{"model":"gemma-4","messages":[{"role":"user","content":"hi"}],"max_tokens":5}' \
  http://127.0.0.1:8080/v1/chat/completions

# (2) Clear BlastGuard index cache
test -d .blastguard && rm -r .blastguard

# (3) Rebuild release binary
cargo build --release 2>&1 | tail -3
```

- [ ] **Step 2: Run the round-12 rollouts**

```bash
bench/.venv/bin/python -m bench.microbench \
  --api-base http://127.0.0.1:8080/v1 \
  --api-key-env DUMMY_KEY \
  --model gemma-4 --model-id-override gemma-4 \
  --tasks chain-search-to-graph outline-tree-sitter-rust find-tamper-patterns \
  --seeds 1 \
  --run-judge --judge-n 3 \
  --output bench/runs/$(date +%Y%m%d-%H%M%S)-round12-chain-to-file.jsonl
```

Expected runtime: 15-25 minutes on Gemma 4.

- [ ] **Step 3: Read the grader summary**

The harness prints a priority-ordered summary at the end. Record these four numbers before closing the terminal:

- Priority 1a grader: `chain-search-to-graph / blastguard` correct? (was `False` in round 11)
- Priority 1a grader: the other two tasks — correctness should be unchanged from round 11 (outline: `True`; find-tamper: `True`).
- Priority 1b judge: per-task winner.
- Priority 2 tokens: BG input-token delta on each task.

- [ ] **Step 4: Append Round 12 section to `docs/MICROBENCH.md`**

Edit `docs/MICROBENCH.md`. At the bottom of the file, append a new section of this exact shape, filling in the measured numbers:

```markdown
## Round 12 — tool-level fix: `chain from X to FILE`

Scope: same 3 tasks × 1 seed × 3 judges as rounds 9–11. The only code
changes between round 11 and this run are:

1. `src/graph/ops.rs` — new `shortest_path_to_predicate` BFS.
2. `src/search/structural.rs::chain_from_to` — path-mode branch that
   walks to the first symbol in the target file and appends sibling
   Calls-successors in that file.
3. `bench/prompts.py` — one-bullet cheat-sheet update noting that `B`
   may be a file path.

Run: `bench/runs/<FILL-IN>-round12-chain-to-file.jsonl`.

| task                        | BG vs raw input | BG vs raw wall | P1a grader     | P1b judge       |
|-----------------------------|:---------------:|:--------------:|:--------------:|:---------------:|
| chain-search-to-graph       | <FILL>          | <FILL>         | <FILL>         | <FILL>          |
| outline-tree-sitter-rust    | <FILL>          | <FILL>         | <FILL>         | <FILL>          |
| find-tamper-patterns        | <FILL>          | <FILL>         | <FILL>         | <FILL>          |

### What the round establishes

<FILL IN — honest read of the numbers. If chain-search P1a flipped
to BG-win, say so with the exact BG answer transcript fragment
showing `structural` is now present. If it didn't flip, say why
(e.g. agent didn't call the new form) and propose the next step.>

### What it does NOT establish

- Single seed is still underpowered for token/wall claims.
- Gemma-specific — a cloud model may use the new form differently.
```

- [ ] **Step 5: Commit the write-up**

```bash
git add docs/MICROBENCH.md
git commit -m "docs/MICROBENCH: round 12 — tool-level fix for chain-search

<one-sentence outcome summary: e.g. 'BG flips P1a win on
chain-search-to-graph; outline and find-tamper unchanged'>

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 6: Judgement call on next step**

After the round-12 write-up lands, two possible next moves:

- **Round 12 shows the P1a flip** → recommend a cloud-model validation round on Opus 4.7 or Sonnet 4.6, then consider shipping. Open a GitHub issue or memory note for that follow-up.
- **Round 12 does NOT show the flip** (the agent didn't discover the file-path form, or discovered it but still stopped early) → inspect the BG trajectory for chain-search-to-graph, identify the specific failure (prompt-not-discoverable vs. tool-still-incomplete), and loop back to brainstorming with the new evidence.

Do not iterate further without presenting findings to the user first.

---

## Self-Review

Checked against the spec and the plan structure:

1. **Spec coverage** — every spec section maps to a task:
   - "Changes §1 graph/ops helper" → Task 1.
   - "Changes §2 structural.rs is_path_like + chain_to_file_path + branch" → Tasks 2+3.
   - "Changes §3 BLASTGUARD_BIAS one-line edit" → Task 4.
   - "Changes §4 query.rs — no change" → no task, as intended.
   - "Testing" — unit tests colocated in each touched file; Tasks 1 and 2 cover the named tests; Task 3 reruns the full suite.
   - "Verification before done" — Task 5 gates (cargo check/test/clippy/build).
   - "Bench re-run + MICROBENCH append" → Task 6.
2. **Placeholder scan** — `<FILL>` tokens in the round-12 table and narrative are intentional; they're the measurement slots filled in at execution time, not design-time placeholders. Everything else (commands, code blocks, commit messages) is complete.
3. **Type/name consistency** — `shortest_path_to_predicate`, `is_path_like`, `chain_to_file_path`, `CHAIN_FILE_CANDIDATE_CAP` used consistently across Tasks 1-3; `chain_from_to` signature unchanged across the edit.
4. **File-path comparison** — all test assertions use `PathBuf::from(...)` and `Path::ends_with` component-aware semantics; no string equality on paths.
5. **TDD discipline** — Tasks 1, 2 write tests first and explicitly assert they fail before implementation lands (Tasks 1 step 2, Task 2 step 3).

No gaps, no contradictions.
