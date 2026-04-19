# BlastGuard Tuning via Micro-bench Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Use `bench/microbench.py` as a regression-style optimization harness to iteratively tune BlastGuard (bundle docstrings, response formatting, empty-hit hints) until the BG arm beats the raw arm on the aggregate — driven entirely by data, not assumption.

**Architecture:** Each task makes ONE focused change, rebuilds the release binary, re-runs the micro-bench, and compares to a locked baseline (round-2 results from `docs/MICROBENCH.md`). Keep changes that improve totals; revert changes that regress. Every round commits its result for traceability.

**Tech Stack:** Rust (BlastGuard src), Python (`bench/microbench.py`), OpenRouter API for MiniMax M2.7, ~$0.07 per micro-bench round × 5 rounds ≈ $0.35 budget.

---

## Baseline (locked)

From `docs/MICROBENCH.md` round 2, MiniMax M2.7 + `BLASTGUARD_BIAS`:

| Arm | Turns | In tok | Out tok | Cost | Wall |
|---|--:|--:|--:|--:|--:|
| raw        | 14 |  78,668 | 3,448 | $0.0277 | 73.6s |
| blastguard | 21 | 117,774 | 3,905 | $0.0400 | 84.5s |
| **Δ (BG − raw)** | **+7** | **+39,106** | **+457** | **+$0.0123** | **+10.9s** |

**Success criterion:** reduce **total BG arm cost** while keeping answer quality subjectively comparable. Final aspirational target: BG cost ≤ raw cost.

**Where the loss comes from (attributed by the round-2 data):**

1. Bundle advertises `chain from X to Y`, cross-file callers, and `importers of FILE` — Phase 1 graph doesn't resolve any of those. Model tries, gets partial/empty results, falls back. Net: extra turns.
2. Tool schema overhead (~1 KB per turn) from three verbose BlastGuard tool descriptions. Every turn pays this cost.
3. On direct-symbol questions (`callers-apply-edit`), the model calls `blastguard_search` *and* `grep` — because the BlastGuard result says `{"hits":[]}` with no hint that grep would be a better next move.

---

## File structure

**Modify:**
- `bench/bundles/blastguard/config.yaml` — tighten tool docstrings; remove over-promised capabilities
- `src/search/structural.rs` — change empty-hit responses to include a brief "try grep" hint
- `src/mcp/server.rs` (or adapter) — trim MCP tool descriptions (the ones the LLM sees) to the minimum necessary
- `docs/MICROBENCH.md` — append a "Tuning rounds" section with the trajectory

**No new files.** Every change is a targeted edit to existing code, followed by a micro-bench re-run.

---

## Task 1: Tighten bundle docstrings — remove over-promised capabilities

**Files:**
- Modify: `bench/bundles/blastguard/config.yaml`

**Why:** Round-2 data shows MiniMax M2.7 calls `blastguard_search '{"query":"chain from X to Y"}'` and cross-file callers, gets partial/empty responses, then burns turns falling back to grep. Removing those capabilities from the advertised surface area stops the wasted attempts.

- [x] **Step 1: Read the current bundle config**

Run: `cat bench/bundles/blastguard/config.yaml`

Note the existing `tools.blastguard_search.docstring` — it mentions "structural relationships (callers, callees, imports, tests, outline)" with `{{...}}` examples including `outline of PATH`, `callers of NAME`, `find NAME`, etc.

- [x] **Step 2: Rewrite `blastguard_search.docstring` to Phase-1-accurate**

Use `Edit` to replace the `blastguard_search` block in `bench/bundles/blastguard/config.yaml`. Replace the existing `docstring` with:

```yaml
  blastguard_search:
    signature: |
      blastguard_search <json_query>
    docstring: >
      AST-graph query over the project. json_query is a single-quoted JSON
      string with a 'query' key. Phase 1 supports four query types reliably:
      '{{"query": "outline of PATH"}}' returns every symbol (name + signature +
      line) in a file — use this instead of reading a whole file to understand
      its shape. '{{"query": "find NAME"}}' returns the file:line where a
      symbol is defined. '{{"query": "exports of PATH"}}' returns only
      visibility-filtered public symbols. '{{"query": "libraries"}}' lists
      external + internal packages the project imports. Best for a first
      pass on a new file. NOTE: callers-of and chain-of-calls queries are
      intra-file only in Phase 1 — for cross-file dependency work, prefer
      grep directly.
    arguments:
      - name: json_query
        type: string
        description: "Single-quoted JSON string with a 'query' key."
        required: true
```

The key differences: explicit "Phase 1 supports four query types reliably", a clear "use outline instead of reading whole file" framing, and an explicit note redirecting cross-file work to grep.

- [x] **Step 3: Rewrite `blastguard_apply_change` docstring to focus on in-file cascade**

```yaml
  blastguard_apply_change:
    signature: |
      blastguard_apply_change <json_changes>
    docstring: >
      Apply edits to a file with in-file cascade-warning analysis. json_changes
      is a single-quoted JSON string like '{{"file": "path", "changes":
      [{{"old_text": "...", "new_text": "..."}}]}}'. Returns SIGNATURE /
      ASYNC_CHANGE / ORPHAN / INTERFACE_BREAK warnings for same-file callers
      plus their signatures. Use this over str_replace_editor when editing
      a function signature or removing a public symbol — the cascade warnings
      flag same-file blast radius immediately. For cross-file blast radius,
      follow up with grep.
    arguments:
      - name: json_changes
        type: string
        description: "Single-quoted JSON with 'file' and 'changes' keys."
        required: true
```

- [x] **Step 4: Rewrite `blastguard_run_tests` docstring (minimal cleanup)**

No behavior change; just trim unnecessary words:

```yaml
  blastguard_run_tests:
    signature: |
      blastguard_run_tests <json_opts>
    docstring: >
      Run the project's test suite (auto-detects pytest/jest/cargo).
      json_opts is a single-quoted JSON string like '{{"path":
      "optional/subpath"}}' or '{{}}' for the default. Failures are annotated
      with "YOU MODIFIED X" when a stack frame hits a recently-edited symbol
      — use after apply_change to verify the cascade didn't break tests.
    arguments:
      - name: json_opts
        type: string
        description: "Single-quoted JSON, 'path' key optional."
        required: false
```

- [x] **Step 5: Verify the bundle still loads**

Run (from repo root):
```bash
export SWE_AGENT_CONFIG_DIR="$(pwd)/bench/.sweagent-repo/config"
export SWE_AGENT_TOOLS_DIR="$(pwd)/bench/.sweagent-repo/tools"
bench/.venv/bin/python -c "
import yaml
from pathlib import Path
c = yaml.safe_load(Path('bench/bundles/blastguard/config.yaml').read_text())
tools = c['tools']
print(f'tools: {sorted(tools.keys())}')
for name, spec in tools.items():
    print(f'  {name}: docstring is {len(spec[\"docstring\"])} chars')
"
```

Expected: three tools listed (`blastguard_apply_change`, `blastguard_run_tests`, `blastguard_search`), each with shorter docstrings than before.

- [x] **Step 6: Commit**

```bash
git add bench/bundles/blastguard/config.yaml
git commit -m "bench: Phase-1-accurate bundle docstrings

Round-2 micro-bench showed the model calling blastguard_search for
chain queries and cross-file callers (which Phase 1 can't resolve),
then wasting turns on the fallback path. Tighten the docstrings to
advertise only the four query types that work reliably in Phase 1:
outline of PATH, find NAME, exports of PATH, libraries. Redirect
cross-file work to grep explicitly.

apply_change docstring narrowed to in-file cascade. run_tests
docstring trimmed for brevity.

No code change; just what the agent sees in its tool schema."
```

---

## Task 2: Re-run micro-bench (round 3) and compare to baseline

**Files:**
- No code changes; measurement task only.

- [ ] **Step 1: Rebuild the release binary**

Run: `cargo build --release 2>&1 | tail -2`
Expected: `Finished` line, no errors. The bundle is loaded at runtime so no rebuild is strictly needed, but we do it for cleanliness.

- [ ] **Step 2: Clear the previous round's microbench.jsonl so we get a clean comparison**

Run:
```bash
test -f bench/results/microbench.jsonl && mv bench/results/microbench.jsonl \
    bench/results/microbench-round2.jsonl
```

Rationale: preserve round 2 under a named file so we can diff round 3 against it.

- [ ] **Step 3: Run the micro-bench**

Run (from repo root):
```bash
export OPENROUTER_API_KEY=$(grep OPENROUTER_API_KEY bench/.env | cut -d= -f2)
bench/.venv/bin/python -m bench.microbench 2>&1 | tee /tmp/round3.log
```

Expected: completes in ~3-5 minutes, spends ~$0.06-0.10, prints a final summary table at the end.

- [ ] **Step 4: Move result to a named file**

Run:
```bash
mv bench/results/microbench.jsonl bench/results/microbench-round3.jsonl
```

- [ ] **Step 5: Compute the delta vs. round 2**

Run:
```bash
bench/.venv/bin/python -c "
import json, pathlib

def summarize(path):
    raw_tokens_in = raw_tokens_out = raw_cost = raw_turns = raw_wall = 0
    bg_tokens_in = bg_tokens_out = bg_cost = bg_turns = bg_wall = 0
    bg_tool_calls = 0
    for line in pathlib.Path(path).open():
        r = json.loads(line)
        if r['arm'] == 'raw':
            raw_tokens_in += r['input_tokens']; raw_tokens_out += r['output_tokens']
            raw_cost += r['total_cost_usd']; raw_turns += r['turns']; raw_wall += r['wall_seconds']
        else:
            bg_tokens_in += r['input_tokens']; bg_tokens_out += r['output_tokens']
            bg_cost += r['total_cost_usd']; bg_turns += r['turns']; bg_wall += r['wall_seconds']
            bg_tool_calls += sum(v for k, v in r['tool_calls'].items() if k.startswith('blastguard_'))
    return {
        'raw_turns': raw_turns, 'raw_in': raw_tokens_in, 'raw_out': raw_tokens_out,
        'raw_cost': raw_cost, 'raw_wall': raw_wall,
        'bg_turns': bg_turns, 'bg_in': bg_tokens_in, 'bg_out': bg_tokens_out,
        'bg_cost': bg_cost, 'bg_wall': bg_wall, 'bg_tool_calls': bg_tool_calls,
    }

r2 = summarize('bench/results/microbench-round2.jsonl')
r3 = summarize('bench/results/microbench-round3.jsonl')

print('                    round 2       round 3       delta')
for k in ('raw_turns','raw_in','raw_out','raw_cost','raw_wall',
          'bg_turns','bg_in','bg_out','bg_cost','bg_wall','bg_tool_calls'):
    v2 = r2[k]; v3 = r3[k]
    d = v3 - v2
    pct = (d / v2 * 100) if v2 else 0
    print(f'{k:<18} {v2:>12.4f}  {v3:>12.4f}  {d:>+10.4f} ({pct:+.1f}%)')
"
```

Expected: a delta table showing how round-3 compares. **Success:** BG cost / turns / tool_calls decrease vs. round 2. **Failure:** they go up — revert Task 1 and try a different approach.

- [ ] **Step 6: Record the outcome**

If BG totals improved: proceed to Task 3.
If BG totals regressed OR tool-call count dropped to 0 (model stopped using BlastGuard entirely): revert the commit from Task 1 with `git revert HEAD` and STOP. Document the finding in `docs/MICROBENCH.md` under a new "Round 3 — regression" subsection.

- [ ] **Step 7: Commit the round-3 data**

```bash
git add bench/results/microbench-round2.jsonl bench/results/microbench-round3.jsonl
git commit -m "bench: round-3 micro-bench after Phase-1-accurate docstrings"
```

Note: `bench/results/` is currently gitignored. This is a deliberate exception — checking in these two JSONL files gives reviewers the raw data to reproduce our numbers without re-running. If you prefer keeping the ignore rule strict, add the files via `git add -f` and update `.gitignore` with specific exceptions for `microbench-round*.jsonl`.

---

## Task 3: Trim MCP tool descriptions (the ones the LLM sees)

**Files:**
- Modify: `src/mcp/server.rs` (look for the `search_tool`, `apply_change_tool`, `run_tests_tool` handlers — their doc strings become the MCP tool descriptions)

**Why:** Round-2 data shows ~1 KB per turn in tool-schema overhead. The current handler doc comments are detailed routing prose ("USE THIS INSTEAD OF native grep when..."). We can compress them 40-60% without losing the routing intent, saving ~500 bytes per turn × 20 turns = 10 KB per task.

- [x] **Step 1: Find the existing tool doc comments**

Run: `grep -n 'USE THIS\|Prefer over\|###' src/mcp/server.rs | head -20`
Inspect the three `#[tool]` handler doc comments for `search`, `apply_change`, and `run_tests`.

- [x] **Step 2: Read the full current doc comments**

Run: `sed -n '/^[[:space:]]*\/\/\/.*search/,/^[[:space:]]*pub async fn search/p' src/mcp/server.rs | head -60`

Note the full text so you can compare before/after.

- [x] **Step 3: Compress the `search` handler's doc comment**

Edit the `///` doc comment above the `search` tool handler in `src/mcp/server.rs`. Replace the current multi-paragraph description with:

```rust
/// Query the project's AST code graph.
///
/// Phase 1 supports: `outline of PATH` (all symbols in a file),
/// `find NAME` (fuzzy symbol lookup), `exports of PATH` (public symbols),
/// `libraries` (external/internal package list), `callers of NAME`
/// (same-file only), and `grep <pattern>` fallback.
///
/// Returns 50-300 tokens of structured graph data instead of 10K+ from raw
/// grep. Prefer this over native Grep for structural queries; use Grep for
/// free-text search across files.
```

Roughly: halves the description length while keeping the routing signal.

- [x] **Step 4: Compress the `apply_change` handler's doc comment**

```rust
/// Apply one or more edits to a file with same-file cascade warnings.
///
/// Returns SIGNATURE / ASYNC_CHANGE / ORPHAN / INTERFACE_BREAK warnings
/// for callers in the same file plus their signatures. Multi-change edits
/// roll back atomically on mid-sequence failure.
///
/// Prefer over native Edit/Write when editing a signature or removing a
/// public symbol — the cascade warnings surface same-file blast radius
/// immediately. For cross-file blast radius, follow up with grep.
```

- [x] **Step 5: Compress the `run_tests` handler's doc comment**

```rust
/// Run the project's test suite (auto-detects jest / vitest / pytest / cargo).
///
/// Failures are annotated with `YOU MODIFIED X (N edits ago)` when a
/// failing stack frame lands inside a symbol the session recently edited
/// via apply_change. Use after an edit to tie regressions to recent work.
```

- [x] **Step 6: Rebuild and run the Rust test suite**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: 253 passed (or current count). Doc-comment changes don't affect test behavior; this is a regression-check.

- [x] **Step 7: Verify the live binary's tool list shows the new shorter descriptions**

Run:
```bash
cargo build --release 2>&1 | tail -2
(
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"s","version":"0"}}}'
  echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
  sleep 1
) | target/release/blastguard /home/adam/Documents/blastguard 2>/dev/null | python3 -c "
import json, sys
for line in sys.stdin:
    r = json.loads(line)
    if r.get('id') == 2:
        for t in r['result']['tools']:
            desc = t.get('description', '')
            print(f\"{t['name']}: {len(desc)} chars\")
"
```

Expected: three tools, each with a description length smaller than before (ballpark: 150-300 chars each, down from 400-600).

- [x] **Step 8: Commit**

```bash
git add src/mcp/server.rs
git commit -m "mcp: compress tool descriptions ~40%

Round-2 micro-bench measured ~1 KB/turn in tool-schema overhead.
Compressing the three handler doc comments (which become the MCP
tool descriptions the LLM sees) cuts ~400 bytes per turn without
losing the routing signal — 20 turns × 400 bytes = 8 KB saved
per task.

253 Rust lib tests still pass."
```

---

## Task 4: Re-run micro-bench (round 4) and compare

**Files:** none; measurement.

- [ ] **Step 1: Re-run**

Exactly as Task 2 Step 3, but tag the output as round 4:

```bash
test -f bench/results/microbench.jsonl && rm bench/results/microbench.jsonl
export OPENROUTER_API_KEY=$(grep OPENROUTER_API_KEY bench/.env | cut -d= -f2)
bench/.venv/bin/python -m bench.microbench 2>&1 | tee /tmp/round4.log
mv bench/results/microbench.jsonl bench/results/microbench-round4.jsonl
```

- [ ] **Step 2: Compute delta vs. round 3 (not round 2)**

Same delta script as Task 2 Step 5, but comparing `microbench-round3.jsonl` to `microbench-round4.jsonl`.

- [ ] **Step 3: Decision**

If BG cost / turns reduced further: proceed to Task 5.
If flat or regressed: commit round-4 data anyway for traceability, then proceed to Task 5 — the wins may compound.

- [ ] **Step 4: Commit**

```bash
git add bench/results/microbench-round4.jsonl
git commit -m "bench: round-4 micro-bench after MCP description compression"
```

---

## Task 5: Empty-hit hints in `callers_of` / `imports_of` / `tests_for`

**Files:**
- Modify: `src/search/structural.rs`

**Why:** When `callers of X` returns no same-file hits, the current response is `{"hits":[]}`. The model then tries more BlastGuard queries before giving up and using grep. Returning a structured hint ("no same-file callers; for cross-file callers use grep or Read") reduces the wasted follow-up turns.

- [x] **Step 1: Find the empty-hit paths**

Run: `grep -n 'return Vec::new' src/search/structural.rs`

Inspect each site. The three relevant ones are `callers_of`, `imports_of`, and `tests_for` — their empty-result paths currently return `Vec::new()`.

- [x] **Step 2: Add a synthetic "hint" SearchHit type variant**

Read `src/search/hit.rs` or wherever `SearchHit` is defined:

Run: `grep -n 'struct SearchHit\|pub struct SearchHit' src/search/`

Add a new constructor on `SearchHit`:

```rust
impl SearchHit {
    /// Synthetic "no-match" hint. Used when a query returns no hits but
    /// there's useful guidance about where the match might live — e.g.
    /// "no same-file callers; try grep for cross-file".
    #[must_use]
    pub fn empty_hint(message: &str) -> Self {
        Self {
            file: std::path::PathBuf::new(),
            line: 0,
            signature: Some(message.to_string()),
            snippet: None,
        }
    }
}
```

- [x] **Step 3: Use the hint in `callers_of`'s empty branches**

In `src/search/structural.rs::callers_of`, change:

```rust
let Some(target_ids) = find_all_by_name(graph, name) else {
    return Vec::new();
};
```

to:

```rust
let Some(target_ids) = find_all_by_name(graph, name) else {
    return vec![SearchHit::empty_hint(
        "no symbol named {name} found; try `find` for fuzzy matches or grep across files"
    )];
};
```

And at the other early-return:

```rust
if hits.is_empty() {
    return vec![SearchHit::empty_hint(
        "no same-file callers in Phase 1 graph; for cross-file callers, use grep",
    )];
}
```

- [x] **Step 4: Add a unit test for the hint path**

In `src/search/structural.rs` tests module, add:

```rust
#[test]
fn callers_of_returns_hint_when_no_callers_found() {
    let mut g = CodeGraph::new();
    g.insert_symbol(Symbol {
        id: SymbolId {
            file: PathBuf::from("a.rs"),
            name: "orphan".to_string(),
            kind: SymbolKind::Function,
        },
        line_start: 1,
        line_end: 2,
        signature: "fn orphan()".to_string(),
        params: vec![],
        return_type: None,
        visibility: Visibility::Export,
        body_hash: 0,
        is_async: false,
        embedding_id: None,
    });
    let hits = callers_of(&g, "orphan", 10);
    assert_eq!(hits.len(), 1);
    let hint = hits[0].signature.as_deref().expect("hint signature");
    assert!(hint.contains("cross-file"));
    assert!(hint.contains("grep"));
}
```

- [x] **Step 5: Run the new test**

Run: `cargo test --lib search::structural::tests::callers_of_returns_hint_when_no_callers_found 2>&1 | tail -5`
Expected: PASS.

- [x] **Step 6: Run full suite**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: N+1 passed (where N was the prior count). If any existing tests fail, they likely asserted `hits.is_empty()` — update them to either check for the hint string or assert `hits.len() == 1 && hits[0].signature.as_deref().unwrap().contains("grep")`.

- [x] **Step 7: Clippy pedantic clean**

Run: `cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3`
Expected: `Finished`, no warnings.

- [x] **Step 8: Commit**

```bash
git add src/search/structural.rs src/search/hit.rs
git commit -m "search: empty-hit hints redirect to grep for cross-file

Round-2 micro-bench showed the model calling blastguard_search for
cross-file callers, getting '{\"hits\":[]}', then trying another
BlastGuard query before falling back to grep. Return a structured
hint on the empty-result path: 'no same-file callers in Phase 1
graph; for cross-file callers, use grep'.

Added callers_of_returns_hint_when_no_callers_found unit test.
254 Rust lib tests pass, clippy pedantic clean."
```

---

## Task 6: Re-run micro-bench (round 5) and compare

**Files:** none; measurement.

- [ ] **Step 1: Rebuild release binary (Rust code changed in Task 5)**

Run: `cargo build --release 2>&1 | tail -2`
Expected: `Finished`.

- [ ] **Step 2: Run the micro-bench**

Exactly as Task 4 Step 1, tagged round 5:

```bash
test -f bench/results/microbench.jsonl && rm bench/results/microbench.jsonl
export OPENROUTER_API_KEY=$(grep OPENROUTER_API_KEY bench/.env | cut -d= -f2)
bench/.venv/bin/python -m bench.microbench 2>&1 | tee /tmp/round5.log
mv bench/results/microbench.jsonl bench/results/microbench-round5.jsonl
```

- [ ] **Step 3: Compute delta vs. round 4**

Same delta script; compare round 4 → round 5.

- [ ] **Step 4: Commit**

```bash
git add bench/results/microbench-round5.jsonl
git commit -m "bench: round-5 micro-bench after empty-hit redirect hints"
```

---

## Task 7: Update `docs/MICROBENCH.md` with the full tuning trajectory

**Files:**
- Modify: `docs/MICROBENCH.md`

- [ ] **Step 1: Append a "Tuning trajectory" section**

Use `Edit` to append, at the very bottom of `docs/MICROBENCH.md`:

```markdown
## Tuning trajectory (iterative micro-bench optimizations)

After establishing round-2 as the baseline, we iterated on BlastGuard
using the micro-bench as a regression harness. Each row is one change
plus a re-run of the same 4 tasks on the same model (MiniMax M2.7 +
BLASTGUARD_BIAS).

| Round | Change | BG cost | BG turns | BG tool calls | Δ cost vs prev |
|---:|---|--:|--:|--:|--:|
| 2 | (baseline — bundle advertised Phase-2 capabilities) | $0.0400 | 21 | 7 | — |
| 3 | Phase-1-accurate bundle docstrings | (fill in)  | (fill in) | (fill in) | (fill in) |
| 4 | Compressed MCP tool descriptions ~40% | (fill in)  | (fill in) | (fill in) | (fill in) |
| 5 | Empty-hit hints redirect to grep on cross-file | (fill in)  | (fill in) | (fill in) | (fill in) |

Pending follow-ups (out of this tuning plan):

- Phase 2 cross-file resolver — the biggest leverage. Would eliminate
  the root cause of the current cross-file-task losses rather than
  just tell the model to fall back.
- Model ablation — Sonnet 4.6 and GLM-5.1 may use tools differently.
- Task diversity — 4 tasks is too few to generalize.

Total optimization spend: $0.xx (fill in — round 3 + 4 + 5 costs).
```

Fill in the (fill in) placeholders from the round-3, round-4, and round-5 delta scripts' output. Use the actual numbers — don't round aggressively.

- [ ] **Step 2: Commit**

```bash
git add docs/MICROBENCH.md
git commit -m "docs: MICROBENCH tuning trajectory across rounds 3-5"
```

---

## Task 8: Push all new commits to origin

**Files:** none; git operation.

- [ ] **Step 1: Confirm clean working tree and ahead count**

Run: `git status && git log --oneline origin/main..HEAD`
Expected: clean tree, the Task 1 / 3 / 5 / 7 code commits, plus the Task 2 / 4 / 6 data commits.

- [ ] **Step 2: Dry-run push**

Run: `git push --dry-run origin main`

- [ ] **Step 3: Push**

Run: `git push origin main`
Expected: commits land on origin.

---

## Self-review

**Spec coverage vs. stated goal:**
- Tighten bundle docstrings → Task 1
- Compress MCP tool descriptions → Task 3
- Empty-hit redirect hints → Task 5
- Measurement between each change → Tasks 2, 4, 6
- Documentation of trajectory → Task 7
- Release → Task 8

**Placeholder scan:** The "(fill in)" entries in Task 7 Step 1 are deliberate — the task engineer fills them with the actual measured values. This is the correct pattern because we can't predict the results; writing fake numbers here would be worse than acknowledging the measurement gap.

**Type consistency:** `SearchHit::empty_hint` is defined in Task 5 Step 2 and used in Task 5 Step 3 — same signature in both. The delta-script Python pseudocode in Task 2 Step 5 uses the same field names (`input_tokens`, `output_tokens`, `total_cost_usd`, `turns`, `wall_seconds`, `tool_calls`) that `bench/microbench.py::RunResult` defines.

**Risk the plan accepts:** if round 3 regresses (unlikely but possible — shorter docstrings might make the model stop using BlastGuard entirely, driving BG tool-calls to zero and reverting round-2 behavior), Task 2 Step 6 explicitly branches to a revert + document path. Subsequent tasks assume round 3 was kept.

---

## Execution

Per project memory ("Subagent-Driven → always"), dispatch fresh subagents per task via `superpowers:subagent-driven-development`. Tasks 2, 4, 6 are measurement tasks that require OpenRouter API spend — each round costs ~$0.07 (total budget ~$0.25 across three rounds). Task 8 pushes to origin; the user has already explicitly authorized that scope in the previous session.
