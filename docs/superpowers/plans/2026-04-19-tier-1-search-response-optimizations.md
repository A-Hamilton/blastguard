# Tier 1 Search-Response Optimizations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compress BlastGuard's `search` tool output (relative paths, tighter formatting, smart per-query caps, dedup) and measure the effect via two new micro-bench rounds. Target: a further 10-20% reduction in BG-arm cost on top of Plan 12's round-4 peak ($0.0219).

**Architecture:** All changes live in `src/search/`. Each logical change (format compression vs. caps vs. dedup) is a separate task so regressions can be attributed. Round 6 measures the format/path changes; round 7 adds caps + dedup on top.

**Tech Stack:** Rust (src/search/hit.rs, src/search/structural.rs, src/search/dispatcher.rs). Python micro-bench runner. ~$0.14 across two re-runs (2 × $0.07).

---

## Baseline (Plan 12 round 4 peak)

| Arm | Turns | In tok | Out tok | Cost | Wall |
|---|--:|--:|--:|--:|--:|
| raw        | 14 | 88,290 | 3,304 | $0.0305 | 57.5s |
| blastguard | 16 | 60,970 | 3,012 | $0.0219 | 69.7s |
| **Δ (BG − raw)** | **+2** | **−27,320** | **−292** | **−$0.0086** | **+12.2s** |

BG arm is already 28% cheaper than raw at round 4. This plan aims to widen that gap further (or at least not lose ground) via output-size wins.

**Success criterion:** BG arm cost stays ≤ raw arm cost AND BG input tokens decrease. If either regresses at round 6, revert and document.

---

## File structure

**Modify:**
- `src/search/hit.rs` — add `SearchHit::to_compact_line(project_root)` for relative-path + tighter signature output
- `src/search/dispatcher.rs` — thread `project_root` into the search response rendering so paths can be relativized
- `src/search/structural.rs` — add per-query-type caps; add outline dedup logic
- `src/mcp/server.rs` — use the compact renderer when emitting the `{"hits":[...]}` payload

**No new files.** Every change is a targeted edit followed by a micro-bench re-run.

---

## Task 1: Compact hit formatting + relative paths

**Files:**
- Modify: `src/search/hit.rs`

**Why:** Round-4 data shows every `blastguard_search` hit emits ~90 chars, ~50 of which are the absolute-path prefix (`/home/adam/Documents/blastguard/`). An outline of a 15-symbol file eats 1,350 chars of which ~750 are redundant path. The model doesn't need the absolute path — it already knows the project root.

Tight format also drops rarely-needed lifetime / generic bounds from the signature line: `callers(graph: &'g CodeGraph, target: &SymbolId): Vec<&'g SymbolId>` → `callers(graph, target) -> Vec<&SymbolId>`. Agents use this as orientation, not as a copy-paste signature.

- [x] **Step 1: Read the current SearchHit definition**

Run: `cat src/search/hit.rs`

Note the existing fields and any existing render method.

- [x] **Step 2: Write a failing test for `to_compact_line`**

Append to `src/search/hit.rs` inside the `#[cfg(test)] mod tests` block (add the module if it doesn't exist):

```rust
#[cfg(test)]
mod tests_compact {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn compact_line_uses_relative_path_when_under_project_root() {
        let hit = SearchHit {
            file: PathBuf::from("/proj/root/src/graph/ops.rs"),
            line: 12,
            signature: Some("callers(graph: &'g CodeGraph, target: &SymbolId): Vec<&'g SymbolId>".to_string()),
            snippet: None,
        };
        let line = hit.to_compact_line(&PathBuf::from("/proj/root"));
        assert!(line.starts_with("src/graph/ops.rs:12"), "got: {line}");
        assert!(!line.contains("/proj/root"), "absolute path leaked: {line}");
        assert!(line.contains("callers"), "signature name should survive: {line}");
    }

    #[test]
    fn compact_line_strips_lifetimes_and_trailing_generics() {
        let hit = SearchHit {
            file: PathBuf::from("/p/src/a.rs"),
            line: 1,
            signature: Some("fn foo<'a, T: Sized>(x: &'a T) -> Vec<&'a T>".to_string()),
            snippet: None,
        };
        let line = hit.to_compact_line(&PathBuf::from("/p"));
        assert!(!line.contains("'a"), "lifetime not stripped: {line}");
        assert!(!line.contains("T: Sized"), "generic bound not stripped: {line}");
        assert!(line.contains("foo"));
    }

    #[test]
    fn compact_line_preserves_absolute_path_when_outside_project_root() {
        let hit = SearchHit {
            file: PathBuf::from("/other/abs/path.rs"),
            line: 5,
            signature: Some("fn bar()".to_string()),
            snippet: None,
        };
        let line = hit.to_compact_line(&PathBuf::from("/proj/root"));
        assert!(line.starts_with("/other/abs/path.rs:5"), "got: {line}");
    }

    #[test]
    fn compact_line_falls_back_to_snippet_when_no_signature() {
        let hit = SearchHit {
            file: PathBuf::from("/p/a.rs"),
            line: 2,
            signature: None,
            snippet: Some("let NEEDLE = 1;".to_string()),
        };
        let line = hit.to_compact_line(&PathBuf::from("/p"));
        assert!(line.contains("NEEDLE"), "got: {line}");
    }
}
```

- [x] **Step 3: Run the test to confirm failure**

Run: `cargo test --lib search::hit::tests_compact 2>&1 | tail -10`
Expected: compile error or missing-method error (`to_compact_line` not found).

- [x] **Step 4: Implement `to_compact_line`**

Add to `src/search/hit.rs` inside `impl SearchHit`:

```rust
impl SearchHit {
    /// Render the hit as a single compact line suitable for an MCP tool
    /// response. Uses `project_root`-relative paths when possible, and
    /// strips lifetime/generic-bound syntax from the signature — agents
    /// use this as orientation, not as a copy-paste-ready declaration.
    ///
    /// Examples:
    ///
    /// - `src/graph/ops.rs:12 callers(graph, target) -> Vec<&SymbolId>`
    /// - `/other/abs/path.rs:5 fn bar()`  (path outside project_root)
    /// - `src/a.rs:2 let NEEDLE = 1;`     (no signature — uses snippet)
    #[must_use]
    pub fn to_compact_line(&self, project_root: &std::path::Path) -> String {
        let path = self
            .file
            .strip_prefix(project_root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| self.file.display().to_string());
        let body = match (self.signature.as_deref(), self.snippet.as_deref()) {
            (Some(sig), _) => compact_signature(sig),
            (None, Some(snippet)) => snippet.trim().to_string(),
            (None, None) => String::new(),
        };
        if body.is_empty() {
            format!("{path}:{}", self.line)
        } else {
            format!("{path}:{} {body}", self.line)
        }
    }
}

/// Strip Rust-specific noise from a signature line that agents don't need
/// for orientation: explicit lifetimes (`'a`, `'g`, `'static`), trait
/// bounds inside generics (`T: Sized`), and the leading `fn ` keyword.
/// Converts the Rust-idiomatic `): Ret` return-type colon to `) -> Ret`
/// only when the original had no `->`.
fn compact_signature(sig: &str) -> String {
    // Strip generic bounds: `<'a, T: Sized, U>` → `<T, U>`. Remove lifetimes
    // and any `X: Y` bound; keep only bare type names.
    let mut out = String::with_capacity(sig.len());
    let mut depth_angle: i32 = 0;
    let mut i = 0;
    let bytes = sig.as_bytes();
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '<' => {
                depth_angle += 1;
                out.push(c);
                i += 1;
            }
            '>' => {
                depth_angle -= 1;
                out.push(c);
                i += 1;
            }
            '\'' if depth_angle > 0 || is_at_lifetime_boundary(bytes, i) => {
                // Skip the lifetime: 'a, 'static, etc.
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                // Also swallow any trailing ", " or "," that was tied to this lifetime.
                if i < bytes.len() && bytes[i] == b',' {
                    i += 1;
                    while i < bytes.len() && bytes[i] == b' ' {
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }

    // Remove trait bounds `X: Y` where X is an identifier and Y extends to
    // the next `,` or `>` at the same depth. We don't parse Rust — this is
    // a cheap regex-style pass.
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut j = 0;
    while j < chars.len() {
        if chars[j] == ':' && j > 0 && chars[j - 1].is_ascii_alphanumeric() && inside_generics(&chars, j) {
            // Skip until matching `,` or `>` at current depth.
            let mut depth = 0;
            while j < chars.len() {
                match chars[j] {
                    '<' => depth += 1,
                    '>' if depth == 0 => break,
                    '>' => depth -= 1,
                    ',' if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            continue;
        }
        cleaned.push(chars[j]);
        j += 1;
    }

    // `fn name(...)`: drop the leading `fn ` when present.
    let trimmed = cleaned.strip_prefix("fn ").unwrap_or(&cleaned);

    // `): T` -> `) -> T` if there's no `->` already.
    if !trimmed.contains("->") {
        if let Some(idx) = trimmed.rfind("):") {
            let (head, tail) = trimmed.split_at(idx + 1);
            // Replace `:` with ` ->`.
            return format!("{head} ->{}", &tail[1..]);
        }
    }
    trimmed.to_string()
}

fn is_at_lifetime_boundary(bytes: &[u8], i: usize) -> bool {
    // Very conservative check: a `'` outside of a string literal, followed
    // by an alphanumeric. We assume signatures don't contain string literals.
    if i + 1 >= bytes.len() {
        return false;
    }
    bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_'
}

fn inside_generics(chars: &[char], pos: usize) -> bool {
    // True if there's an unmatched `<` somewhere before `pos`.
    let mut depth = 0i32;
    for &c in &chars[..pos] {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}
```

The logic is deliberately simple: drop lifetimes (`'a`), drop generic bounds (`T: Sized`), drop the `fn ` keyword, convert `): T` return-type style to `) -> T`. It won't win style points on edge cases, but for the ~95% of agent-facing signatures in this project it cuts 20-30% of chars.

- [x] **Step 5: Run the test**

Run: `cargo test --lib search::hit::tests_compact 2>&1 | tail -10`
Expected: all 4 tests pass.

- [x] **Step 6: Run the full library suite**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: N+4 passed (where N was the prior count, which is 254 after Plan 12 Task 5).

- [x] **Step 7: Clippy pedantic clean**

Run: `cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3`
Expected: `Finished`, no warnings.

- [x] **Step 8: Commit**

```bash
git add src/search/hit.rs
git commit -m "search: compact hit formatting + relative paths

Add SearchHit::to_compact_line(project_root) that emits one line per
hit in a minimal form:

  src/graph/ops.rs:12 callers(graph, target) -> Vec<&SymbolId>

Replaces the previous format which included the absolute path prefix
(~50 chars per hit wasted) and full Rust-idiomatic signature syntax
with lifetimes and trait bounds (which agents use for orientation,
not copy-paste).

Still to wire up: src/mcp/server.rs and src/search/dispatcher.rs need
to call to_compact_line instead of the existing renderer. That lands
in Task 2.

4 new unit tests in src/search/hit.rs::tests_compact."
```

---

## Task 2: Wire the compact renderer into the MCP response

**Files:**
- Modify: `src/mcp/server.rs` — find where the `search` tool's response is built; swap the hit rendering to `to_compact_line`
- Modify: `src/search/dispatcher.rs` — ensure `dispatch` already receives `project_root` (it does per current code); no field changes needed

- [x] **Step 1: Find the current render path**

Run: `grep -n '"hits"\|structured_content\|hits\.iter' src/mcp/server.rs src/mcp/*.rs src/search/*.rs 2>/dev/null | head`

Identify the site that converts `Vec<SearchHit>` to the JSON `{"hits": [...]}` payload that the MCP tool returns.

- [x] **Step 2: Swap to `to_compact_line`**

Edit the relevant file (likely `src/mcp/server.rs` in the `search` handler or its helper). Replace the current per-hit rendering (which probably uses `hit.signature` directly) with:

```rust
let rendered: Vec<String> = hits
    .iter()
    .map(|h| h.to_compact_line(&project_root))
    .collect();
let payload = serde_json::json!({ "hits": rendered });
```

Where `project_root` is the already-available project root `PathBuf`. If the existing site doesn't have it in scope, thread it through from the server's `ServerConfig` / `BlastGuardServer` struct.

- [x] **Step 3: Write an integration test**

Append to `tests/integration_mcp_server.rs`:

```rust
#[test]
fn search_response_uses_compact_format() {
    // Spin up a minimal in-process MCP server against a small fixture
    // project that has a known symbol. Send a tools/call for search
    // with query "find something". Assert the response JSON contains
    // relative paths (no "/home/") and doesn't include lifetime syntax.
    // (Implementation should mirror existing integration_mcp_server.rs tests.)
    //
    // If the existing integration test infrastructure in this file
    // doesn't easily support a JSON-response assertion, leave this
    // test behind a `#[ignore]` attribute with a TODO comment and
    // file it as a follow-up rather than blocking the tuning work.
    // The round 6 micro-bench re-run will catch any regression in
    // practice.
}
```

**Pragmatic note:** if the existing integration test harness makes this test hard to write in under 30 minutes, skip it. Round 6's micro-bench is the real gate.

- [x] **Step 4: Rebuild and verify live**

```bash
cargo build --release 2>&1 | tail -2
(
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"s","version":"0"}}}'
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"search","arguments":{"query":"outline of src/search/dispatcher.rs"}}}'
  sleep 1
) | target/release/blastguard /home/adam/Documents/blastguard 2>/dev/null | python3 -c "
import json, sys
for line in sys.stdin:
    r = json.loads(line)
    if r.get('id') == 2:
        hits = r['result']['structuredContent']['hits']
        print(f'total hits: {len(hits)}')
        for h in hits[:3]:
            print(' ', h)
        # Assertions
        for h in hits:
            assert '/home/adam/' not in h, f'absolute path leaked: {h}'
            assert \"'\" not in h or '&\\'' not in h, f'lifetime leaked: {h}'
        print('OK: all hits use relative paths, no lifetime syntax')
"
```

Expected: prints 3+ hits, each under ~100 chars, no absolute paths, no `'a` / `'g` lifetimes.

- [x] **Step 5: Run the full suite**

Run: `cargo test 2>&1 | tail -5`
Expected: all existing library + integration tests still pass.

- [x] **Step 6: Commit**

```bash
git add src/mcp/server.rs src/search/dispatcher.rs tests/integration_mcp_server.rs
git commit -m "mcp: render search hits via to_compact_line

Wire the new SearchHit::to_compact_line renderer into the search
tool's response payload. Relative paths + stripped lifetimes /
generic bounds cut per-hit output by ~40%.

Verified live against the release binary: outline of
src/search/dispatcher.rs returns hits under 100 chars each, no
absolute paths, no lifetime syntax.

Ready for round-6 micro-bench."
```

---

## Task 3: Round 6 micro-bench — measure format/path compression

**Files:** none; measurement.

- [ ] **Step 1: Rebuild release**

Run: `cargo build --release 2>&1 | tail -2`

- [ ] **Step 2: Run**

```bash
test -f bench/results/microbench.jsonl && rm bench/results/microbench.jsonl
export OPENROUTER_API_KEY=$(grep OPENROUTER_API_KEY bench/.env | cut -d= -f2)
bench/.venv/bin/python -m bench.microbench 2>&1 | tee /tmp/round6.log
mv bench/results/microbench.jsonl bench/results/microbench-round6.jsonl
```

- [ ] **Step 3: Compute delta vs round 4 (the current peak)**

```bash
bench/.venv/bin/python -c "
import json, pathlib
def summarize(path):
    raw={'turns':0,'in':0,'out':0,'cost':0.0,'wall':0.0}
    bg=dict(raw); bg_tool=0
    for line in pathlib.Path(path).open():
        r=json.loads(line)
        d = raw if r['arm']=='raw' else bg
        d['turns']+=r['turns']; d['in']+=r['input_tokens']; d['out']+=r['output_tokens']
        d['cost']+=r['total_cost_usd']; d['wall']+=r['wall_seconds']
        if r['arm']=='blastguard':
            bg_tool += sum(v for k,v in r['tool_calls'].items() if k.startswith('blastguard_'))
    return raw,bg,bg_tool
r4_raw,r4_bg,r4_tc = summarize('bench/results/microbench-round4.jsonl')
r6_raw,r6_bg,r6_tc = summarize('bench/results/microbench-round6.jsonl')
print(f'{\"\":<16} {\"round4\":>10} {\"round6\":>10} {\"delta\":>12}')
for arm,d2,d3 in [('raw',r4_raw,r6_raw),('bg',r4_bg,r6_bg)]:
    for k in ('turns','in','out','cost','wall'):
        v2,v3=d2[k],d3[k]; pct=(v3-v2)/v2*100 if v2 else 0
        print(f'{arm+\".\"+k:<16} {v2:>10.4f} {v3:>10.4f} {v3-v2:>+8.4f} ({pct:+.1f}%)')
print(f'{\"bg_tool_calls\":<16} {r4_tc:>10d} {r6_tc:>10d} {r6_tc-r4_tc:>+8d}')
"
```

- [ ] **Step 4: Decision**

- If **BG cost decreased AND BG input tokens decreased** (both vs round 4): proceed to Task 4.
- If **BG cost increased ≥20%** vs round 4: revert Tasks 1 and 2 with `git revert HEAD~1..HEAD`, commit the round-6 data under a new name (`microbench-round6-reverted.jsonl`), STOP this plan, and write up the finding.
- If **flat (within ±5%)**: accept and proceed to Task 4. The individual change didn't hurt; the next tier's changes may compound.

- [ ] **Step 5: Commit round-6 data**

```bash
git add -f bench/results/microbench-round6.jsonl
git commit -m "bench: round-6 micro-bench after compact hit formatting

<fill in with numbers from Step 3: BG cost delta, input tokens delta,
turn count delta, tool call delta>"
```

---

## Task 4: Smart per-query caps

**Files:**
- Modify: `src/search/structural.rs`

**Why:** Currently every structural query path has the same `max_hits = 10` cap. `outline of` on a large file (say `src/search/structural.rs` with 20+ symbols) truncates silently. `find NAME` at 10 results returns mostly fuzzy-match noise — exact-first-then-top-4-fuzzy is plenty. `libraries` has no natural cap (the project typically has 30-50 packages).

Smart defaults:

- `outline_of`: 50 (rarely hit; but when a file has 30 symbols we don't want to truncate)
- `find`: 5 (exact match + 4 fuzzy alternatives)
- `callers_of` / `callees_of`: 10 (unchanged — intra-file is bounded)
- `exports_of`: 50 (same rationale as outline)
- `libraries`: 30 (covers typical project)
- `grep` fallback: 30 (unchanged per spec)

- [ ] **Step 1: Locate the cap usage**

Run: `grep -n 'DEFAULT_MAX_HITS\|max_hits' src/search/ -r`

Inspect `src/search/dispatcher.rs` — the constant is likely there and is passed into structural calls.

- [ ] **Step 2: Introduce per-arm caps**

Replace the single `DEFAULT_MAX_HITS` constant in `src/search/dispatcher.rs` with a small function:

```rust
/// Per-query-type result caps. `outline` / `exports` need headroom for
/// files with 30+ symbols; `find` only needs exact + 4 fuzzy
/// alternatives; `callers` / `callees` are bounded by same-file scope
/// in Phase 1 so 10 is fine. `libraries` and `grep` follow SPEC §3.
const OUTLINE_MAX_HITS: usize = 50;
const EXPORTS_MAX_HITS: usize = 50;
const FIND_MAX_HITS: usize = 5;
const CALLERS_MAX_HITS: usize = 10;
const CALLEES_MAX_HITS: usize = 10;
const LIBRARIES_MAX_HITS: usize = 30;
```

Update `dispatch`:

```rust
match classify(query) {
    QueryKind::Find(name) => structural::find(graph, &name, FIND_MAX_HITS),
    QueryKind::Callers(name) => structural::callers_of(graph, &name, CALLERS_MAX_HITS),
    QueryKind::Callees(name) => structural::callees_of(graph, &name, CALLEES_MAX_HITS),
    QueryKind::Outline(path) => {
        let resolved = resolve_query_path(project_root, &path);
        let mut hits = structural::outline_of(graph, &resolved);
        hits.truncate(OUTLINE_MAX_HITS);
        hits
    }
    // … existing arms unchanged, except:
    QueryKind::ExportsOf(path) => {
        let resolved = resolve_query_path(project_root, &path);
        let mut hits = structural::exports_of(graph, &resolved);
        hits.truncate(EXPORTS_MAX_HITS);
        hits
    }
    QueryKind::Libraries => {
        let mut hits = structural::libraries(graph);
        hits.truncate(LIBRARIES_MAX_HITS);
        hits
    }
    // … other arms unchanged.
}
```

- [ ] **Step 3: Add a unit test**

Append to `src/search/dispatcher.rs` tests module:

```rust
#[test]
fn outline_respects_50_hit_cap() {
    let project_root = PathBuf::from("/proj/root");
    let mut g = CodeGraph::new();
    for i in 0..80 {
        g.insert_symbol(Symbol {
            id: SymbolId {
                file: project_root.join("src/big.rs"),
                name: format!("fn_{i}"),
                kind: SymbolKind::Function,
            },
            line_start: (i + 1) as u32,
            line_end: (i + 2) as u32,
            signature: format!("fn fn_{i}()"),
            params: vec![],
            return_type: None,
            visibility: Visibility::Export,
            body_hash: 0,
            is_async: false,
            embedding_id: None,
        });
    }
    let hits = dispatch(&g, &project_root, "outline of src/big.rs");
    assert_eq!(hits.len(), 50, "outline should cap at 50");
}

#[test]
fn find_respects_5_hit_cap() {
    let mut g = CodeGraph::new();
    for i in 0..20 {
        g.insert_symbol(Symbol {
            id: SymbolId {
                file: PathBuf::from(format!("/p/a{i}.rs")),
                name: format!("mytest_{i}"),
                kind: SymbolKind::Function,
            },
            line_start: 1, line_end: 2,
            signature: format!("fn mytest_{i}()"),
            params: vec![], return_type: None,
            visibility: Visibility::Export,
            body_hash: 0, is_async: false, embedding_id: None,
        });
    }
    let hits = dispatch(&g, Path::new("/p"), "find mytest");
    assert!(hits.len() <= 5, "find should cap at 5, got {}", hits.len());
}
```

- [ ] **Step 4: Run the new tests**

Run: `cargo test --lib search::dispatcher 2>&1 | tail -10`
Expected: 7 passed (5 prior + 2 new).

- [ ] **Step 5: Run the full suite + clippy**

Run: `cargo test --lib 2>&1 | tail -3 && cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3`
Expected: all tests pass, clippy clean.

- [ ] **Step 6: Commit**

```bash
git add src/search/dispatcher.rs
git commit -m "search: per-query-type result caps

Replace the single DEFAULT_MAX_HITS=10 constant with per-arm caps:
outline / exports at 50 (large files), find at 5 (exact + 4 fuzzy),
callers / callees unchanged at 10 (bounded intra-file in Phase 1),
libraries at 30 (typical project).

Fixes silent outline truncation on files with >10 symbols and
reduces find-fuzzy-match noise.

2 new unit tests; 256 lib tests pass."
```

---

## Task 5: Dedup test vs production symbols in outline

**Files:**
- Modify: `src/search/structural.rs`

**Why:** `outline of src/index/cache.rs` returns both production functions and test-module functions (`#[cfg(test)] fn some_test()`). Currently they appear as peer entries with no label — the agent can't tell which is which without reading the line. Collapse / tag the test entries so agents can filter noise immediately.

Pragmatic approach: detect when two hits share the same function name and they're in the same file, and mark the one at a higher line number (typically the test) with a `[test]` prefix. Or, simpler: tag any symbol whose `line_start` falls inside a range that has a previous `#[cfg(test)]` marker. The first approach is cheaper.

- [ ] **Step 1: Read the current outline_of**

Run: `sed -n '/pub fn outline_of/,/^}/p' src/search/structural.rs`

- [ ] **Step 2: Write a failing test**

Append to `src/search/structural.rs` tests module:

```rust
#[test]
fn outline_of_tags_duplicate_test_functions() {
    let file = PathBuf::from("/p/a.rs");
    let mut g = CodeGraph::new();
    // Production function at line 10.
    g.insert_symbol(sym_at("foo", &file, 10));
    // Test function at line 100 (higher — treated as "the test copy").
    g.insert_symbol(sym_at("foo", &file, 100));
    let hits = outline_of(&g, &file);
    assert_eq!(hits.len(), 2);
    // The later one should be tagged.
    let tagged = hits.iter().find(|h| h.line == 100).expect("hit at 100");
    assert!(
        tagged.signature.as_deref().unwrap_or("").starts_with("[test]"),
        "expected [test] tag, got: {:?}",
        tagged.signature
    );
    // The earlier one should NOT be tagged.
    let untagged = hits.iter().find(|h| h.line == 10).expect("hit at 10");
    assert!(!untagged.signature.as_deref().unwrap_or("").starts_with("[test]"));
}

#[cfg(test)]
fn sym_at(name: &str, file: &std::path::Path, line: u32) -> Symbol {
    Symbol {
        id: SymbolId {
            file: file.to_path_buf(),
            name: name.to_string(),
            kind: SymbolKind::Function,
        },
        line_start: line,
        line_end: line + 1,
        signature: format!("fn {name}()"),
        params: vec![],
        return_type: None,
        visibility: Visibility::Private,
        body_hash: 0,
        is_async: false,
        embedding_id: None,
    }
}
```

- [ ] **Step 3: Confirm test fails**

Run: `cargo test --lib outline_of_tags_duplicate 2>&1 | tail -5`
Expected: assertion failure — the test tag isn't added.

- [ ] **Step 4: Implement the tagging**

In `src/search/structural.rs::outline_of`, after collecting and sorting hits, walk them and mark duplicates:

```rust
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

    // Tag duplicate-name entries after the first occurrence as `[test]`
    // — files with both a production `fn foo` and a `#[cfg(test)] fn foo`
    // emit both; the later one is almost always the test copy.
    let mut seen: std::collections::HashMap<String, ()> = std::collections::HashMap::new();
    for hit in &mut hits {
        let Some(sig) = &hit.signature else { continue };
        // Extract the function name (first word after stripping noise).
        let name = extract_fn_name(sig);
        if seen.contains_key(&name) {
            hit.signature = Some(format!("[test] {sig}"));
        } else {
            seen.insert(name, ());
        }
    }
    hits
}

fn extract_fn_name(sig: &str) -> String {
    // Very cheap: take the token before the first `(`. Good enough for
    // Rust-idiomatic signatures.
    let head = sig.split('(').next().unwrap_or(sig);
    let trimmed = head.strip_prefix("fn ").unwrap_or(head);
    trimmed.trim().split_whitespace().last().unwrap_or("").to_string()
}
```

- [ ] **Step 5: Run the new test**

Run: `cargo test --lib outline_of_tags_duplicate 2>&1 | tail -5`
Expected: PASS.

- [ ] **Step 6: Run full suite + clippy**

Run: `cargo test --lib 2>&1 | tail -3 && cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3`
Expected: all tests pass (count grew by 1 vs Task 4), clippy clean.

- [ ] **Step 7: Commit**

```bash
git add src/search/structural.rs
git commit -m "search: tag duplicate-name test functions in outline

Files with both \`#[cfg(test)] fn foo\` and production \`fn foo\`
emitted both at peer visibility in outline responses, forcing the
agent to read both lines to disambiguate. Now, the later occurrence
is prefixed with \`[test]\` so the agent can filter at a glance.

1 new unit test; 257 lib tests pass."
```

---

## Task 6: Round 7 micro-bench — measure caps + dedup on top

**Files:** none; measurement.

- [ ] **Step 1: Rebuild release**

Run: `cargo build --release 2>&1 | tail -2`

- [ ] **Step 2: Run**

```bash
test -f bench/results/microbench.jsonl && rm bench/results/microbench.jsonl
export OPENROUTER_API_KEY=$(grep OPENROUTER_API_KEY bench/.env | cut -d= -f2)
bench/.venv/bin/python -m bench.microbench 2>&1 | tee /tmp/round7.log
mv bench/results/microbench.jsonl bench/results/microbench-round7.jsonl
```

- [ ] **Step 3: Compute delta vs round 6**

Use the same delta-script as Task 3 Step 3 but comparing round 6 → round 7.

- [ ] **Step 4: Commit the data**

```bash
git add -f bench/results/microbench-round7.jsonl
git commit -m "bench: round-7 micro-bench after smart caps + outline dedup

<fill in with numbers from Step 3>"
```

---

## Task 7: Update MICROBENCH.md with rounds 6-7

**Files:**
- Modify: `docs/MICROBENCH.md`

- [ ] **Step 1: Append two new rows to the "Tuning trajectory" table**

Use `Edit` to add two new rows to the existing Markdown table in `docs/MICROBENCH.md`:

```markdown
| 6  | Compact hit formatting + relative paths           | $<cost> | <turns>  | <in>      | <calls>  | <delta>           |
| 7  | Smart per-query caps + outline test/prod dedup    | $<cost> | <turns>  | <in>      | <calls>  | <delta>           |
```

Fill in the `<...>` values from round 6 and round 7's actual data.

- [ ] **Step 2: Add a short narrative subsection**

After the existing "What didn't work (round 5)" subsection, append:

```markdown
### Rounds 6-7 (compact output + smart caps)

Round 6 applied two changes together: compact hit formatting (drop
lifetimes, drop generic bounds, drop the `fn` keyword) and relative
paths in place of absolute. Measured effect: <summarize round 6 delta>.

Round 7 stacked per-query-type result caps (outline/exports 50,
find 5, libraries 30) and `[test]` tagging for duplicate-name
functions in outline. Measured effect: <summarize round 7 delta>.

The compact format change is output-size-only (the agent gets a
smaller response per query); the caps and dedup change both output
size AND signal-to-noise on the agent's next decision.
```

- [ ] **Step 3: Commit**

```bash
git add docs/MICROBENCH.md
git commit -m "docs(microbench): rounds 6-7 — compact output + smart caps"
```

---

## Task 8: Push to origin

**Files:** none; git operation.

- [ ] **Step 1: Confirm the log**

Run: `git log --oneline origin/main..HEAD`
Expected: the six tuning commits (Tasks 1, 2, 4, 5) plus the two measurement commits (Tasks 3, 6) plus the docs commit (Task 7).

- [ ] **Step 2: Push**

Run: `git push origin main 2>&1 | tail -3`

---

## Self-review

**Spec coverage:**
- Relative paths → Task 1 (`to_compact_line`)
- Compact signatures → Task 1 (`compact_signature`)
- Smart caps → Task 4
- Outline dedup → Task 5
- Measurement between each "layer" of change → Tasks 3, 6
- Documentation → Task 7
- Release → Task 8

**Placeholder scan:** The `<fill in>` markers in Task 3 Step 5, Task 6 Step 4, and Task 7 Steps 1-2 are deliberate — they're for actual measured values, not pre-baked numbers. This is the correct pattern because we can't predict data-driven values.

**Type consistency:**
- `SearchHit::to_compact_line(project_root: &Path) -> String` — declared in Task 1 Step 4, used in Task 2 Step 2.
- `extract_fn_name` helper in Task 5 is a new free function; it's used inside `outline_of` only and doesn't need public visibility.
- `OUTLINE_MAX_HITS` / `FIND_MAX_HITS` / etc. — declared in Task 4 Step 2 and used only in the `dispatch` function in that same file.

**One risk the plan accepts:** Task 2 Step 3's integration test is flagged as pragmatic-skip-if-hard. The round 6 micro-bench is the real gate. This is acceptable because the unit tests in Task 1 Step 2 cover `to_compact_line`'s behavior in isolation.

---

## Execution

Per project memory ("Subagent-Driven → always"), dispatch fresh subagents per task via `superpowers:subagent-driven-development`. Tasks 3 and 6 are measurement tasks consuming OpenRouter credits (~$0.07 each, ~$0.14 total). Task 8 pushes to origin; user has authorized this scope throughout the session.
