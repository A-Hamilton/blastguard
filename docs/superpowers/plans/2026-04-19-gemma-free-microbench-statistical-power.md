# Gemma-4 Free Micro-bench + Statistical Power Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Leverage the local Gemma-4 26B server (`http://127.0.0.1:8080/v1`, zero marginal cost per request) to run the micro-bench with n=8+ tasks × 3+ seeds — enough statistical power to attribute individual-change effects that got swamped by n=4 noise in Plans 12-13. Produces a statistically-defensible `docs/MICROBENCH.md` update and a reusable multi-seed runner.

**Architecture:** Extend `bench/microbench.py` with a `--api-base` / `--api-key-env` pair (so it works against any OpenAI-compatible endpoint, not just OpenRouter), a `--seeds` flag that runs each `(task, arm)` pair N times, and a post-run statistics module that reports means, standard deviations, and 95% CIs. Expand `TASKS` from 4 to 10 to hit the harder multi-file cases that showed up as losses in Plan 12's round-2 analysis. Keep all prior rounds in `bench/results/` for traceability.

**Tech Stack:** Python 3.12 + OpenAI SDK (OpenAI-compatible — already a dep), Rust's `target/release/blastguard` binary, `scipy.stats` (already in `bench/pyproject.toml`) for CIs, local Gemma-4 26B via llama-server on port 8080.

---

## Prerequisites verified

- Gemma-4 endpoint reachable: `curl http://127.0.0.1:8080/v1/models` returns `{"data":[{"id":"gemma-4",...}]}`.
- Tool calling confirmed working with clean JSON args (tested with `calc` function).
- Thinking mode is ON by default (emits `reasoning_content`); that's fine, our loop already ignores unknown fields.
- Context window: 32K (set via `CTX=32768` override, within the 16 GB GPU budget at `NCMOE=20`).
- Speed: ~52 tok/s generation, ~185 tok/s prompt processing. A 20-turn task ≈ 15-25 min. Full run ≈ 4-8 hours — free, but slow.

---

## File structure

**Modify:**
- `bench/microbench.py` — add `--api-base`, `--api-key-env`, `--seeds`, `--model-id-override` flags; accept Gemma's `reasoning_content` field without crashing; skip cost math when prices are zero.
- `bench/pyproject.toml` — no change (scipy already present).

**Create:**
- `bench/stats_aggregate.py` — read one-or-more `microbench-*.jsonl` files, compute per-arm per-task means / std / 95% CIs, output a Markdown table.
- `bench/tasks_registry.py` — extracted TASKS list with the existing 4 + 6 new tasks (harder multi-file). Keeps microbench.py small.
- `bench/tests/test_stats_aggregate.py` — 3 unit tests for the aggregator.

**Commit in `bench/results/`:**
- `microbench-gemma-smoke.jsonl` (Task 2)
- `microbench-gemma-seed1.jsonl` / `-seed2.jsonl` / `-seed3.jsonl` (Task 5)

---

## Task 1: Endpoint + seed + price flags in microbench.py

**Files:**
- Modify: `bench/microbench.py`

**Why:** Today, the script hard-codes `base_url="https://openrouter.ai/api/v1"` and `api_key=os.environ["OPENROUTER_API_KEY"]`. To point it at Gemma locally we need: (a) configurable base_url and api-key env var, (b) a way to set per-token prices to 0 for local models so cost logs don't lie.

- [x] **Step 1: Read the current argparse block**

Run: `sed -n '/def main/,/^    args = p.parse_args/p' bench/microbench.py | head -30`

Note that `--in-price` / `--out-price` / `--cache-price` already exist. We need to add `--api-base`, `--api-key-env`, `--seeds`, and plumb them through.

- [x] **Step 2: Add the new flags and plumb into run_task**

Open `bench/microbench.py`. In `main()`'s argparse block, before `args = p.parse_args()`, add:

```python
    p.add_argument(
        "--api-base",
        default="https://openrouter.ai/api/v1",
        help="OpenAI-compatible API base URL (set to http://127.0.0.1:8080/v1 for local Gemma)",
    )
    p.add_argument(
        "--api-key-env",
        default="OPENROUTER_API_KEY",
        help="Env var name to read the API key from. Local servers usually accept any value; still required to be set.",
    )
    p.add_argument(
        "--seeds",
        type=int,
        default=1,
        help="Run each (task, arm) pair this many times with seeds 1..N. "
             "Extra seeds give us variance estimates for stats_aggregate.py.",
    )
    p.add_argument(
        "--model-id-override",
        default=None,
        help="Override the model ID sent in the chat/completions request while keeping "
             "the --model value in the output log. Use when the local endpoint expects "
             "a short ID (e.g. 'gemma-4') but you want the log tagged with the full name.",
    )
```

Then in `run_task`, change the OpenAI client construction to use these flags:

```python
from openai import OpenAI  # noqa: PLC0415

client = OpenAI(
    api_key=os.environ.get(api_key_env, "not-needed-for-local"),
    base_url=api_base,
)
```

And propagate by adding two parameters to `run_task`'s keyword-only signature:
```python
def run_task(
    *,
    task: dict[str, str],
    arm: str,
    model: str,
    project_root: str,
    blastguard_binary: str,
    max_turns: int = 25,
    in_price: float = 0.30,
    out_price: float = 1.20,
    apply_bias: bool = True,
    api_base: str = "https://openrouter.ai/api/v1",
    api_key_env: str = "OPENROUTER_API_KEY",
    model_id_for_api: str | None = None,  # if set, use this in the request instead of `model`
    seed_value: int = 1,  # reproducibility marker in the output record
) -> RunResult:
```

Inside `run_task`, use `model_id_for_api if model_id_for_api is not None else model` for the API call, but keep `model` in the `RunResult.task_id`/`arm` payload unchanged.

- [x] **Step 3: Wire the seed loop**

Replace the current `for task in TASKS: for arm in ("raw", "blastguard"):` block in `main()` with:

```python
    for seed_idx in range(1, args.seeds + 1):
        for task in TASKS:
            for arm in ("raw", "blastguard"):
                print(f"\n=== task={task['id']} arm={arm} seed={seed_idx} ===")
                r = run_task(
                    task=task,
                    arm=arm,
                    model=args.model,
                    project_root=args.project_root,
                    blastguard_binary=args.blastguard_binary,
                    max_turns=args.max_turns,
                    in_price=args.in_price,
                    out_price=args.out_price,
                    api_base=args.api_base,
                    api_key_env=args.api_key_env,
                    model_id_for_api=args.model_id_override,
                    seed_value=seed_idx,
                )
                print(
                    f"  seed={seed_idx} turns={r.turns} in={r.input_tokens} "
                    f"out={r.output_tokens} cost=${r.total_cost_usd:.4f} wall={r.wall_seconds:.1f}s "
                    f"stop={r.stopped_reason}"
                )
                print(f"  tools: {r.tool_calls}")
                print(f"  answer (first 200 chars): {r.final_answer[:200]!r}")
                results.append(r)
```

- [x] **Step 4: Add a `seed` field to RunResult**

Update the `RunResult` dataclass (search `@dataclass ... class RunResult`) to add:

```python
    seed: int
```

Set it from `seed_value` inside `run_task` right before returning:

```python
    return RunResult(
        task_id=task["id"],
        arm=arm,
        seed=seed_value,
        turns=turn_count,
        # … existing fields unchanged
    )
```

- [x] **Step 5: Make cost math tolerate zero prices**

Zero prices mean a local model (Gemma). The existing `input_cost = total_in * in_price / 1_000_000.0` already produces 0.0 in that case — no code change needed, just document it in a docstring line next to `in_price`:

```python
    in_price: float = 0.30,  # USD per M input tokens; set to 0.0 for local models
```

- [x] **Step 6: Smoke-test the arg parsing**

Run:
```bash
cd /home/adam/Documents/blastguard
bench/.venv/bin/python -m bench.microbench --help 2>&1 | grep -E 'api-base|seeds|model-id-override'
```

Expected: 3 new lines describing `--api-base`, `--seeds`, `--model-id-override`. If any is missing, the argparse block edit didn't land.

- [x] **Step 7: Commit**

```bash
git add bench/microbench.py
git commit -m "microbench: add --api-base, --seeds, --model-id-override flags

Enables running the micro-bench against any OpenAI-compatible endpoint,
not just OpenRouter. Key motivator: local Gemma-4 at
http://127.0.0.1:8080/v1 runs tasks for free, unlocking n >= 8 task
runs and multi-seed runs that were cost-prohibitive on paid APIs.

Also:
- --seeds N re-runs each (task, arm) pair N times for variance estimates
- --model-id-override decouples the logged model name from the API
  identifier (useful when local server expects 'gemma-4' but we want
  the log tagged 'ggml-org/gemma-4-26B-A4B-it-GGUF')
- Added RunResult.seed for reproducibility

No output-format change when used with defaults — backwards compatible
with the round 3-6 OpenRouter runs."
```

---

## Task 2: 1-task Gemma smoke — verify end-to-end pipeline

**Files:** None; a single command + assertion.

**Why:** Before investing in task-set expansion and multi-seed runs, confirm Gemma-4 can drive the micro-bench at all: does it call BlastGuard tools with well-formed JSON? Does 32K context hold? Does the harness record what we expect?

- [ ] **Step 1: Verify Gemma is running**

Run: `~/bin/ai status 2>&1 | head -3`
Expected: `llama-swap.service     active`. If `inactive`, run `~/bin/ai llm` first.

- [ ] **Step 2: Narrow TASKS to just one for this smoke**

Temporarily edit `bench/microbench.py` to comment out all but the first task:

```python
TASKS = [
    {
        "id": "explore-cold-index",
        "prompt": (
            "Using the tools available, explore the BlastGuard Rust codebase at "
            "{project_root} and explain what the `cold_index` function does and "
            "what calls it. Answer in 3-5 sentences. When done, write 'DONE' "
            "on its own line."
        ),
    },
    # Temporarily disabled for the Gemma smoke — re-enabled in Task 3.
    # {"id": "callers-apply-edit", ...},
    # {"id": "chain-search-to-graph", ...},
    # {"id": "cascade-signature-change", ...},
]
```

- [ ] **Step 3: Run the smoke**

```bash
cd /home/adam/Documents/blastguard
export OPENAI_API_KEY=not-needed-for-local
bench/.venv/bin/python -m bench.microbench \
    --api-base http://127.0.0.1:8080/v1 \
    --api-key-env OPENAI_API_KEY \
    --model ggml-org/gemma-4-26B-A4B-it-GGUF \
    --model-id-override gemma-4 \
    --in-price 0 --out-price 0 --cache-price 0 \
    --max-turns 30 2>&1 | tee /tmp/gemma-smoke.log
```

Expected within ~20-30 min: the summary table prints at the end with 2 rows (raw arm, blastguard arm), both with `finish=done_marker` or `finish=finish_stop`, non-empty `final_answer`, and at least 1 tool call each.

- [ ] **Step 4: Verify the output is usable**

Run:
```bash
bench/.venv/bin/python -c "
import json
path = 'bench/results/microbench.jsonl'
rows = [json.loads(l) for l in open(path)]
assert len(rows) == 2, f'expected 2 rows got {len(rows)}'
for r in rows:
    assert r['final_answer'], f'{r[\"arm\"]} had empty answer'
    assert r['turns'] > 0
    assert r['total_cost_usd'] == 0.0, 'local model should cost 0'
print('smoke OK')
print('raw   tools:', rows[0]['tool_calls'])
print('BG    tools:', rows[1]['tool_calls'])
print('raw   turns:', rows[0]['turns'])
print('BG    turns:', rows[1]['turns'])
"
```

Expected: `smoke OK` plus the tool-call dicts. If BG arm's tool_calls dict doesn't contain any `blastguard_*` entries, Gemma isn't reaching for BlastGuard — note this as a finding but don't abort; the bias prompt might just need a different phrasing for Gemma.

- [ ] **Step 5: Move the smoke output so Task 3's runs don't overwrite it**

```bash
mv bench/results/microbench.jsonl bench/results/microbench-gemma-smoke.jsonl
```

- [ ] **Step 6: Restore TASKS to the full 4-task list**

Revert the temporary edit from Step 2 — uncomment the 3 tasks you commented out. The plan uses Task 3 to expand this further; don't skip the revert or Task 3 will start from the wrong baseline.

- [ ] **Step 7: Commit the smoke result**

```bash
git add -f bench/results/microbench-gemma-smoke.jsonl
git commit -m "bench: Gemma-4 1-task smoke on microbench

Verifies the local endpoint drives the full microbench pipeline
end-to-end (OpenAI-compatible tool calling, trajectory logging,
JSONL output). Zero API cost.

Results to inspect:
- Turn counts (vs round-6's M2.7 baseline)
- Whether Gemma reaches for BlastGuard without prompt adjustment
- Any unexpected finish_reason values

This smoke is the gate on Tasks 3-5 — if Gemma can't tool-call
cleanly here, those tasks would need to fall back to MiniMax M2.7."
```

---

## Task 3: Expand the task set from 4 to 10

**Files:**
- Create: `bench/tasks_registry.py`
- Modify: `bench/microbench.py`

**Why:** The core limit on Plans 12-13's conclusions was n=4 tasks, 1 seed each. The variance-to-signal ratio is so high that round-to-round deltas under ±20% are noise. Expanding to 10 tasks plus Task 4's seed loop gives the power to detect smaller real effects.

The 6 new tasks cover deliberately diverse patterns: one more intra-file exploration (BlastGuard's sweet spot), two cross-file investigations (where Phase 1 loses), two "modify without breaking" edits (cascade territory), and one test-suite-oriented question (`run_tests` attribution).

- [ ] **Step 1: Create `bench/tasks_registry.py` with the full 10-task list**

```python
"""Centralized registry of micro-bench tasks.

Each task is a dict with `id` (stable identifier for result tables) and
`prompt` (a format-string with `{project_root}` as the only placeholder).

Design: tasks are chosen to span BlastGuard's strengths and weaknesses.
Plans 12-13 found that BG wins on intra-file outline/find and loses on
cross-file dependency chains. The expanded set keeps that balance so
aggregate wins are meaningful.
"""

from __future__ import annotations

TASKS: list[dict[str, str]] = [
    # --- Existing 4 tasks (kept for continuity with rounds 2-6) ---
    {
        "id": "explore-cold-index",
        "prompt": (
            "Using the tools available, explore the BlastGuard Rust codebase at "
            "{project_root} and explain what the `cold_index` function does and "
            "what calls it. Answer in 3-5 sentences. When done, write 'DONE' "
            "on its own line."
        ),
    },
    {
        "id": "callers-apply-edit",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, find every caller of "
            "the `apply_edit` function. For each caller, briefly describe what "
            "it is (function name + file) and what kind of value it passes for "
            "the `old_text` argument. Answer concisely. Write 'DONE' when finished."
        ),
    },
    {
        "id": "chain-search-to-graph",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, find the call chain "
            "from the MCP `search` tool entry point down into the code-graph "
            "module. In other words: when the MCP search tool is invoked, which "
            "intermediate function(s) get called on the way to the graph "
            "operations? Name each function (file + function name) in order. "
            "Keep the answer under 10 lines. Write 'DONE' when finished."
        ),
    },
    {
        "id": "cascade-signature-change",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, suppose we wanted "
            "to change the signature of `apply_edit` to take a single `Edit` "
            "struct instead of three separate `&Path`, `&str`, `&str` "
            "arguments. List every function that would need to be updated, "
            "and explain why. Keep the answer concise — just a bulleted list "
            "with the file:line of each caller and a one-line reason. "
            "Write 'DONE' when finished."
        ),
    },
    # --- Six new tasks (added in Plan 14 Task 3) ---
    {
        # Clean intra-file exploration — should favor BlastGuard outline.
        "id": "outline-tree-sitter-rust",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, list every public "
            "function defined in `src/parse/rust.rs` (file path relative to "
            "project root) with its signature. Group them by category (parsing "
            "entry points, helper utilities, edge emitters). Write 'DONE' when "
            "finished."
        ),
    },
    {
        # Cross-file investigation — currently a BlastGuard weakness in Phase 1.
        "id": "trace-cache-persistence",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, explain how the "
            "BLAKE3 Merkle cache is persisted to disk. Identify: (1) where the "
            "cache gets written, (2) where it gets read back, and (3) the "
            "format it's serialized in. Answer in under 8 sentences. Write "
            "'DONE' when finished."
        ),
    },
    {
        # Easy find + grep task — direct-symbol question where grep usually wins.
        "id": "find-tamper-patterns",
        "prompt": (
            "In the BlastGuard Python harness at {project_root}/bench, list "
            "every filename pattern that counts as benchmark tampering under "
            "the BenchJack defense. Where is this list defined? Answer in 2-3 "
            "lines. Write 'DONE' when finished."
        ),
    },
    {
        # Refactor-lite scoping — caller graph + test impact question.
        "id": "impact-of-removing-libraries",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, suppose we wanted "
            "to remove support for the `libraries` query type from the search "
            "dispatcher. List every file that would need to change, and "
            "describe what the change would look like in each. Keep the answer "
            "concise — bulleted list format. Write 'DONE' when finished."
        ),
    },
    {
        # Multi-file orientation + compare — no single clear tool winner.
        "id": "compare-parse-modules",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, compare the parse "
            "drivers in `src/parse/python.rs` and `src/parse/rust.rs`. What is "
            "the same between them (structure-wise), and what is meaningfully "
            "different? Keep the comparison to 6 sentences or fewer. Write "
            "'DONE' when finished."
        ),
    },
    {
        # Tests-for style question — exercises BlastGuard's run_tests or its
        # structural tests-for query depending on Phase 1 capability.
        "id": "tests-for-apply-change",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, identify every "
            "test function that exercises the `apply_change` or `apply_edit` "
            "code paths. Give the test function name and its file:line. Keep "
            "the answer concise — bulleted list. Write 'DONE' when finished."
        ),
    },
]
```

- [ ] **Step 2: Update microbench.py to import TASKS from the registry**

In `bench/microbench.py`, replace the inline `TASKS = [...]` block with:

```python
from bench.tasks_registry import TASKS
```

Keep the import near the top, after the standard-lib imports and before the native-tool implementations.

- [ ] **Step 3: Add a unit test confirming the registry loads**

Create `bench/tests/test_tasks_registry.py`:

```python
"""Smoke test that TASKS has the expected shape."""

from __future__ import annotations

from bench.tasks_registry import TASKS


def test_registry_has_10_tasks():
    assert len(TASKS) == 10


def test_every_task_has_id_and_prompt_with_placeholder():
    ids: set[str] = set()
    for t in TASKS:
        assert "id" in t and t["id"], f"missing id: {t}"
        assert "prompt" in t and t["prompt"], f"missing prompt: {t['id']}"
        assert "{project_root}" in t["prompt"], (
            f"task {t['id']!r} prompt missing {{project_root}} placeholder"
        )
        assert t["id"] not in ids, f"duplicate id: {t['id']}"
        ids.add(t["id"])


def test_task_ids_are_filesystem_safe():
    # We use ids as dict keys in downstream aggregation; catch sneaky chars now.
    for t in TASKS:
        assert all(c.isalnum() or c in "-_" for c in t["id"]), (
            f"task id contains unsafe character: {t['id']!r}"
        )
```

- [ ] **Step 4: Run the tests**

Run: `cd bench && HF_HOME=/tmp/hf uv run pytest tests/test_tasks_registry.py -v`
Expected: 3 PASS.

- [ ] **Step 5: Verify the microbench still loads without error**

Run: `bench/.venv/bin/python -c "from bench.microbench import TASKS; print(len(TASKS), 'tasks loaded'); [print(' ', t['id']) for t in TASKS]"`
Expected: `10 tasks loaded` followed by all 10 ids.

- [ ] **Step 6: Commit**

```bash
git add bench/tasks_registry.py bench/microbench.py bench/tests/test_tasks_registry.py
git commit -m "bench: expand microbench task set from 4 to 10

Moves TASKS into bench/tasks_registry.py for reuse and adds 6 new
tasks covering patterns under-represented in the original set:

- outline-tree-sitter-rust       (intra-file outline, BG sweet spot)
- trace-cache-persistence        (cross-file investigation)
- find-tamper-patterns           (direct-symbol grep-favoring)
- impact-of-removing-libraries   (caller graph + blast radius)
- compare-parse-modules          (multi-file orientation)
- tests-for-apply-change         (tests-for style)

Rationale: Plan 12-13's n=4 tasks gave us a signal-to-noise ratio too
low to attribute individual-change effects reliably. 10 tasks doubles
the per-arm datapoints; combined with Task 4's --seeds flag, this
unlocks statistical power for small real effects.

Added 3 unit tests (test_tasks_registry.py) to catch registry drift."
```

---

## Task 4: Statistical aggregation module

**Files:**
- Create: `bench/stats_aggregate.py`
- Create: `bench/tests/test_stats_aggregate.py`

**Why:** Manual eyeballing of round deltas at n=4 was the core methodological weakness of Plans 12-13. With multi-seed / multi-task data we need proper aggregation: mean, stddev, 95% CI per `(task, arm)` cell, and arm-level aggregates that respect the paired structure (same tasks, same seeds, only the arm differs).

- [ ] **Step 1: Write the failing tests**

Create `bench/tests/test_stats_aggregate.py`:

```python
"""Unit tests for the multi-seed micro-bench aggregator."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from bench.stats_aggregate import (
    aggregate_per_cell,
    arm_totals_with_ci,
    load_runs,
)


def _write_jsonl(path: Path, records: list[dict]) -> None:
    with path.open("w", encoding="utf-8") as f:
        for r in records:
            f.write(json.dumps(r) + "\n")


def _mk(task_id: str, arm: str, seed: int, cost: float, turns: int) -> dict:
    return {
        "task_id": task_id,
        "arm": arm,
        "seed": seed,
        "turns": turns,
        "input_tokens": turns * 1000,
        "cached_input_tokens": 0,
        "output_tokens": turns * 100,
        "wall_seconds": turns * 2.0,
        "tool_calls": {},
        "final_answer": "x",
        "stopped_reason": "done_marker",
        "input_cost_usd": cost * 0.2,
        "output_cost_usd": cost * 0.8,
        "total_cost_usd": cost,
    }


def test_load_runs_reads_multiple_files(tmp_path: Path):
    f1 = tmp_path / "a.jsonl"
    f2 = tmp_path / "b.jsonl"
    _write_jsonl(f1, [_mk("t1", "raw", 1, 0.01, 3)])
    _write_jsonl(f2, [_mk("t1", "raw", 2, 0.02, 4)])
    runs = load_runs([f1, f2])
    assert len(runs) == 2
    assert {r["seed"] for r in runs} == {1, 2}


def test_aggregate_per_cell_computes_mean_std(tmp_path: Path):
    # Same (task, arm), two seeds; mean cost should be 0.015, std ~ 0.005.
    f = tmp_path / "x.jsonl"
    _write_jsonl(
        f,
        [_mk("t1", "raw", 1, 0.01, 3), _mk("t1", "raw", 2, 0.02, 4)],
    )
    runs = load_runs([f])
    agg = aggregate_per_cell(runs)
    cell = agg[("t1", "raw")]
    assert abs(cell["cost_mean"] - 0.015) < 1e-9
    assert abs(cell["cost_std"] - 0.005) < 1e-9  # population std, n=2
    assert cell["n"] == 2


def test_arm_totals_computes_paired_ci(tmp_path: Path):
    # Raw arm total cost across 3 seeds of 1 task: 0.01, 0.015, 0.02  -> mean 0.015.
    # BG arm across same seeds: 0.008, 0.010, 0.012 -> mean 0.010.
    # With paired differences [0.002, 0.005, 0.008] mean 0.005, the arm
    # totals_with_ci should show bg < raw, and the paired-difference CI
    # should not include 0 at 95% (the fixture is deterministic so we
    # assert a positive lower bound).
    f = tmp_path / "y.jsonl"
    records = []
    for seed, (raw_cost, bg_cost) in enumerate(
        [(0.01, 0.008), (0.015, 0.010), (0.02, 0.012)], start=1
    ):
        records.append(_mk("t1", "raw", seed, raw_cost, 3))
        records.append(_mk("t1", "blastguard", seed, bg_cost, 3))
    _write_jsonl(f, records)
    runs = load_runs([f])
    totals = arm_totals_with_ci(runs)
    assert totals["raw"]["cost_mean"] > totals["blastguard"]["cost_mean"]
    assert "paired_diff" in totals
    # Lower bound of 95% CI on paired (raw - bg) is > 0 -> BG meaningfully cheaper.
    assert totals["paired_diff"]["ci95_low"] > 0.0
```

- [ ] **Step 2: Confirm the tests fail**

Run: `cd bench && uv run pytest tests/test_stats_aggregate.py -v`
Expected: all 3 FAIL with `ModuleNotFoundError: bench.stats_aggregate`.

- [ ] **Step 3: Implement `bench/stats_aggregate.py`**

```python
"""Multi-seed / multi-task micro-bench aggregation.

Reads one or more JSONL files (each line is a `RunResult` as emitted by
`bench/microbench.py`) and produces:

- `load_runs(paths)` -> `list[dict]` — flat list of records from all files
- `aggregate_per_cell(runs)` -> `dict[(task_id, arm), metrics]` — mean,
  std, n, and min/max for cost / input_tokens / turns / wall_seconds
- `arm_totals_with_ci(runs)` -> aggregate per arm with a paired-difference
  95% CI on `(raw − blastguard)` cost. If only one seed is present the CI
  width will be NaN and downstream consumers should treat that row as
  "single draw, no variance estimate".

Uses only stdlib + `statistics` for means/std and `scipy.stats.t` for
the paired CI (already a bench dep via `bench/pyproject.toml`).
"""

from __future__ import annotations

import json
import math
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any

from scipy.stats import t as student_t


def load_runs(paths: list[Path]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for path in paths:
        with Path(path).open("r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                out.append(json.loads(line))
    return out


def _population_std(values: list[float]) -> float:
    # Population std matches numpy/statistics defaults for small n; fine
    # because we're descriptive, not inferential at the cell level.
    if len(values) < 2:
        return 0.0
    return statistics.pstdev(values)


def aggregate_per_cell(
    runs: list[dict[str, Any]],
) -> dict[tuple[str, str], dict[str, float | int]]:
    groups: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for r in runs:
        groups[(r["task_id"], r["arm"])].append(r)

    out: dict[tuple[str, str], dict[str, float | int]] = {}
    for (task_id, arm), rows in groups.items():
        costs = [r["total_cost_usd"] for r in rows]
        ins = [r["input_tokens"] for r in rows]
        turns = [r["turns"] for r in rows]
        walls = [r["wall_seconds"] for r in rows]
        out[(task_id, arm)] = {
            "n": len(rows),
            "cost_mean": statistics.fmean(costs),
            "cost_std": _population_std(costs),
            "cost_min": min(costs),
            "cost_max": max(costs),
            "input_mean": statistics.fmean(ins),
            "turns_mean": statistics.fmean(turns),
            "wall_mean": statistics.fmean(walls),
        }
    return out


def arm_totals_with_ci(runs: list[dict[str, Any]]) -> dict[str, Any]:
    """Per-arm totals across (task, seed) combinations + paired-difference CI.

    The paired structure is: for each (task_id, seed), raw and blastguard
    arms should both have a run. We sum each arm's cost across tasks for
    each seed, then compute a paired t-CI on (raw_total − bg_total) across
    seeds. When n_seeds == 1, the CI width is NaN and we warn.
    """
    # Index by (task_id, seed) -> {arm: cost_dict}
    by_key: dict[tuple[str, int], dict[str, dict]] = defaultdict(dict)
    for r in runs:
        by_key[(r["task_id"], r["seed"])][r["arm"]] = r

    # Per-seed totals.
    seeds = sorted({seed for (_, seed) in by_key})
    per_seed_totals: dict[str, dict[int, float]] = {"raw": {}, "blastguard": {}}
    for seed in seeds:
        raw_cost = 0.0
        bg_cost = 0.0
        for (task_id, s), arms in by_key.items():
            if s != seed:
                continue
            if "raw" in arms:
                raw_cost += arms["raw"]["total_cost_usd"]
            if "blastguard" in arms:
                bg_cost += arms["blastguard"]["total_cost_usd"]
        per_seed_totals["raw"][seed] = raw_cost
        per_seed_totals["blastguard"][seed] = bg_cost

    raw_costs = list(per_seed_totals["raw"].values())
    bg_costs = list(per_seed_totals["blastguard"].values())

    out: dict[str, Any] = {
        "seeds": seeds,
        "n_seeds": len(seeds),
        "raw": {
            "cost_mean": statistics.fmean(raw_costs) if raw_costs else 0.0,
            "cost_std": _population_std(raw_costs),
        },
        "blastguard": {
            "cost_mean": statistics.fmean(bg_costs) if bg_costs else 0.0,
            "cost_std": _population_std(bg_costs),
        },
    }

    if len(seeds) < 2:
        out["paired_diff"] = {
            "mean": (out["raw"]["cost_mean"] - out["blastguard"]["cost_mean"]),
            "ci95_low": float("nan"),
            "ci95_high": float("nan"),
            "note": "single seed — no variance estimate available",
        }
        return out

    diffs = [per_seed_totals["raw"][s] - per_seed_totals["blastguard"][s] for s in seeds]
    n = len(diffs)
    mean = statistics.fmean(diffs)
    sd = statistics.stdev(diffs)  # sample std for inference
    t_crit = student_t.ppf(0.975, df=n - 1)
    half = t_crit * sd / math.sqrt(n)
    out["paired_diff"] = {
        "mean": mean,
        "ci95_low": mean - half,
        "ci95_high": mean + half,
        "per_seed_diffs": diffs,
    }
    return out


def render_markdown_report(runs: list[dict[str, Any]]) -> str:
    """Convenience wrapper that emits the Markdown section we paste into
    `docs/MICROBENCH.md` after a run.
    """
    cells = aggregate_per_cell(runs)
    totals = arm_totals_with_ci(runs)
    lines: list[str] = []
    lines.append("### Per-task means across seeds\n")
    lines.append("| task | arm | n | cost mean | cost std | turns mean | wall mean |")
    lines.append("|---|---|--:|--:|--:|--:|--:|")
    for (task_id, arm), c in sorted(cells.items()):
        lines.append(
            f"| {task_id} | {arm} | {c['n']} | "
            f"${c['cost_mean']:.4f} | ${c['cost_std']:.4f} | "
            f"{c['turns_mean']:.1f} | {c['wall_mean']:.1f}s |"
        )
    lines.append("")
    lines.append("### Arm totals with paired 95% CI on cost difference")
    lines.append("")
    lines.append(f"- seeds run: {totals['seeds']}")
    lines.append(f"- raw arm total cost (mean across seeds): ${totals['raw']['cost_mean']:.4f} "
                 f"(std ${totals['raw']['cost_std']:.4f})")
    lines.append(f"- BG arm total cost (mean across seeds): ${totals['blastguard']['cost_mean']:.4f} "
                 f"(std ${totals['blastguard']['cost_std']:.4f})")
    pd = totals["paired_diff"]
    low = pd.get("ci95_low")
    high = pd.get("ci95_high")
    if low is not None and not math.isnan(low):
        lines.append(
            f"- paired (raw − BG) mean: ${pd['mean']:.4f}, "
            f"95% CI [${low:.4f}, ${high:.4f}]"
        )
        if low > 0:
            lines.append(f"  **BG is cheaper than raw at 95% confidence.**")
        elif high < 0:
            lines.append(f"  **BG is more expensive than raw at 95% confidence.**")
        else:
            lines.append(f"  CI crosses zero — no statistically significant difference.")
    else:
        lines.append(
            f"- paired (raw − BG) mean: ${pd['mean']:.4f} "
            f"({pd.get('note', 'single seed')})"
        )
    return "\n".join(lines)
```

- [ ] **Step 4: Run the tests**

Run: `cd bench && uv run pytest tests/test_stats_aggregate.py -v`
Expected: all 3 PASS.

- [ ] **Step 5: Lint clean**

Run: `cd bench && uv run ruff check bench/stats_aggregate.py bench/tests/test_stats_aggregate.py`
Expected: `All checks passed!`

- [ ] **Step 6: Commit**

```bash
git add bench/stats_aggregate.py bench/tests/test_stats_aggregate.py
git commit -m "bench: stats_aggregate module — means, stds, paired CIs

Reads one or more RunResult JSONL files (optionally from multiple
seeds) and computes per-cell descriptive stats plus arm-level totals
with a paired-t 95% CI on (raw − blastguard) cost.

This is the aggregator the multi-seed Gemma runs feed into. Without
it, Plans 12-13-style 'BG cost went from X to Y, that's a Z% win'
rounds lack a confidence interval — readers have no way to tell
stochastic-n=4 noise from real effects.

3 unit tests cover load/cell-aggregation/paired-CI computation on
deterministic fixtures."
```

---

## Task 5: Full-task-set Gemma run (3 seeds)

**Files:** None; this is an operational task.

**Why:** Task 1-4 built the tooling. Task 5 produces the first real statistically-powered dataset.

- [ ] **Step 1: Sanity check all prerequisites**

```bash
cd /home/adam/Documents/blastguard
# Gemma up
curl -fsS http://127.0.0.1:8080/v1/models > /dev/null && echo "gemma ok" || (echo "start gemma: ~/bin/ai llm"; exit 1)
# Release binary current
cargo build --release 2>&1 | tail -1
# Bench deps up to date
cd bench && uv sync 2>&1 | tail -3
cd ..
# Tasks registry importable
bench/.venv/bin/python -c "from bench.tasks_registry import TASKS; assert len(TASKS) == 10"
# Stats aggregator importable
bench/.venv/bin/python -c "from bench.stats_aggregate import aggregate_per_cell"
echo "all prereqs ok"
```

Expected: `all prereqs ok` as the last line. If any check fails, fix before proceeding — this run takes hours and nothing worse than getting 4 hours in and finding a typo.

- [ ] **Step 2: Warm-up run (1 task, 1 seed) to pre-load Gemma's KV cache**

```bash
rm -f bench/results/microbench.jsonl
export OPENAI_API_KEY=not-needed-for-local
bench/.venv/bin/python -m bench.microbench \
    --api-base http://127.0.0.1:8080/v1 \
    --api-key-env OPENAI_API_KEY \
    --model gemma-4-26b-a4b-it \
    --model-id-override gemma-4 \
    --in-price 0 --out-price 0 --cache-price 0 \
    --seeds 1 --max-turns 30
mv bench/results/microbench.jsonl bench/results/microbench-gemma-warmup.jsonl
```

This single pass (10 tasks × 2 arms × 1 seed = 20 runs, ~2-4 hours on this hardware) does two things: validates the expanded task set against Gemma, and pre-populates KV cache so subsequent seeds are faster.

- [ ] **Step 3: Check the warmup output before committing to Step 4**

```bash
bench/.venv/bin/python -c "
import json
rows = [json.loads(l) for l in open('bench/results/microbench-gemma-warmup.jsonl')]
by_task = {}
for r in rows:
    by_task.setdefault(r['task_id'], {})[r['arm']] = r
print(f'{\"task\":<32} {\"raw_turns\":>10} {\"bg_turns\":>10} {\"bg_tools\":>10}')
for tid, arms in sorted(by_task.items()):
    raw = arms.get('raw', {})
    bg = arms.get('blastguard', {})
    bg_tc = sum(v for k,v in bg.get('tool_calls', {}).items() if k.startswith('blastguard_'))
    print(f'{tid:<32} {raw.get(\"turns\",0):>10} {bg.get(\"turns\",0):>10} {bg_tc:>10}')
"
```

Expected output: every task has both arms present, BG tool counts are > 0 for at least half the tasks. If an arm is missing for some task or BG calls are 0 across the board, stop and debug (likely the bias prompt isn't transferring to Gemma).

- [ ] **Step 4: Full 3-seed run**

```bash
rm -f bench/results/microbench.jsonl
bench/.venv/bin/python -m bench.microbench \
    --api-base http://127.0.0.1:8080/v1 \
    --api-key-env OPENAI_API_KEY \
    --model gemma-4-26b-a4b-it \
    --model-id-override gemma-4 \
    --in-price 0 --out-price 0 --cache-price 0 \
    --seeds 3 --max-turns 30 2>&1 | tee /tmp/gemma-3seed.log
mv bench/results/microbench.jsonl bench/results/microbench-gemma-3seed.jsonl
```

This is a 60-run operation (10 tasks × 2 arms × 3 seeds). On Gemma at ~52 tok/s, expect **6-12 hours**. Run it overnight or in a screen session. Zero API cost.

- [ ] **Step 5: Commit the data**

```bash
git add -f bench/results/microbench-gemma-warmup.jsonl bench/results/microbench-gemma-3seed.jsonl
git commit -m "bench: Gemma-4 3-seed run across 10 tasks — statistically powered dataset

Runs the expanded TASKS registry (10 tasks) × 2 arms × 3 seeds = 60
total runs against local Gemma-4-26B. Zero API cost, ~6-12 hours wall
time on the dev workstation.

This is the first micro-bench dataset with enough power for proper
paired-t CIs on (raw − blastguard). Plans 12-13's n=4 × 1 seed
conclusions get re-examined in Task 6 against this data.

Raw per-run records in microbench-gemma-3seed.jsonl; warmup pass (1
seed) in microbench-gemma-warmup.jsonl (kept separately so it's not
averaged in)."
```

---

## Task 6: Statistical analysis + MICROBENCH.md writeup

**Files:**
- Modify: `docs/MICROBENCH.md`

**Why:** Task 5 produces the data; this task turns it into an honest claim readers can cite.

- [ ] **Step 1: Run the aggregator and render the Markdown**

```bash
cd /home/adam/Documents/blastguard
bench/.venv/bin/python -c "
from bench.stats_aggregate import load_runs, render_markdown_report
from pathlib import Path
runs = load_runs([Path('bench/results/microbench-gemma-3seed.jsonl')])
print(render_markdown_report(runs))
" > /tmp/gemma-report.md
cat /tmp/gemma-report.md
```

- [ ] **Step 2: Inspect the numbers honestly**

Before pasting into the docs, read the output carefully. Answer in the eventual writeup:

- Is the paired (raw − BG) CI's lower bound positive? If yes, BG is cheaper at 95%. If it crosses zero, say so explicitly — the headline is "no statistically significant difference" not "BG wins".
- Which per-task cells show BG >> raw (CodeCompass "hidden-dependency" side) vs. BG ≈ raw (semantic side)?
- Are there any cells where raw beats BG convincingly? Those are worth naming — they point at real Phase 2 work.

- [ ] **Step 3: Append a new "Rounds 6-7 → Gemma" section to `docs/MICROBENCH.md`**

Use `Edit` to append at the end of `docs/MICROBENCH.md`:

```markdown

## Round 8 — Gemma-4 26B, n=10 tasks × 3 seeds

After Plan 13's conclusion that n=4 × 1 seed was too noisy for clean
attribution, Plan 14 added a local Gemma-4 endpoint to the bench
(zero per-request cost) and expanded the task set to 10. This is
the first statistically powered snapshot of the BG-vs-raw comparison.

Model: `ggml-org/gemma-4-26B-A4B-it-GGUF` served via llama-server on
port 8080, 32K context, temperature 0.0, 3 seeds per (task, arm) pair.
All 60 runs used the round-6 BlastGuard binary (commit <fill in with
`git rev-parse HEAD` at the time of the run).

<paste stats_aggregate output here — the full "Per-task means" table
plus "Arm totals with paired 95% CI" section>

### What this dataset tells us that n=4 × 1 couldn't

- <fill in 2-3 sentences summarizing the biggest actionable finding,
  e.g. "BG is cheaper than raw at 95% confidence with a paired mean
  difference of $X ± Y" OR "The paired CI crosses zero, so we can't
  claim an aggregate cost win — but on the 4 hidden-dependency tasks
  BG is ahead by Z cents at 95% while it's behind by W cents on the
  semantic tasks.">

### What this dataset still does NOT tell us

- **Cross-model generality.** Everything here is Gemma-4 behavior.
  MiniMax M2.7 was the prior benchmark target (Plans 12-13); the
  numbers don't transfer directly.
- **SWE-bench Pro lift.** Repo-navigation tasks on the BlastGuard repo
  itself are not multi-file bug fixes on unknown repositories. See
  `bench/KNOWN_GAPS.md` Gap 5 for why a real Pro run is still blocked
  upstream.
- **Answer quality.** We measure cost and turns, not correctness.
  Manual spot-checking of the `final_answer` field in the JSONL is
  the only quality signal — none of these tasks have a ground truth.

### Reproducibility

```bash
~/bin/ai llm                             # start local Gemma
cd /home/adam/Documents/blastguard
cargo build --release
export OPENAI_API_KEY=not-needed-for-local
bench/.venv/bin/python -m bench.microbench \
    --api-base http://127.0.0.1:8080/v1 \
    --api-key-env OPENAI_API_KEY \
    --model gemma-4-26b-a4b-it \
    --model-id-override gemma-4 \
    --in-price 0 --out-price 0 --cache-price 0 \
    --seeds 3
```

Cost: $0. Wall time: 6-12 hours on the dev workstation.
```

Fill in the `<...>` placeholders from `/tmp/gemma-report.md` and the honest interpretation in Step 2.

- [ ] **Step 4: Commit**

```bash
git add docs/MICROBENCH.md
git commit -m "docs(microbench): Gemma-4 3-seed powered analysis

Statistically-powered replacement for the round 3-6 single-draw
conclusions. n=10 tasks × 3 seeds + paired 95% CI + per-cell breakdown.

Captures the headline finding and the task-level split that matches
the CodeCompass hidden-dependency / semantic prediction."
```

---

## Task 7: Push to origin

**Files:** None; git operation.

- [ ] **Step 1: Confirm clean tree and ahead count**

```bash
cd /home/adam/Documents/blastguard
git status
git log --oneline origin/main..HEAD
```

Expected: clean tree; the commits from Tasks 1-6 present.

- [ ] **Step 2: Dry-run push**

```bash
git push --dry-run origin main
```

- [ ] **Step 3: Push**

```bash
git push origin main 2>&1 | tail -3
```

---

## Self-review

**Spec coverage vs. the goal:**

- "Free Gemma endpoint usable" → Task 1 (--api-base / --api-key-env flags).
- "n ≥ 8 tasks" → Task 3 (expanded to 10 in `tasks_registry.py`).
- "Multi-seed for variance" → Task 1 (--seeds) + Task 4 (stats).
- "Statistically defensible update" → Task 6 (paired CI in MICROBENCH.md).

**Placeholder scan:**

- Task 6 Step 3 has `<fill in ...>` markers for the model-commit SHA and the data-driven interpretation. These are deliberate because we can't predict the actual numbers; the surrounding prose tells the engineer exactly what to compute to fill them in. This matches Plan 12/13's convention for measurement-derived values.

**Type / name consistency:**

- `RunResult.seed` added in Task 1 Step 4, consumed in Task 4's `load_runs` and `aggregate_per_cell`. Field name matches across tasks.
- `TASKS` module path `bench.tasks_registry.TASKS` matches in Task 3 Step 1 (creation), Task 3 Step 2 (import in microbench.py), and Task 3 Step 3 (test import).
- Stats module exports three names — `load_runs`, `aggregate_per_cell`, `arm_totals_with_ci` — all three are tested in Task 4 Step 1 and used in Task 6 Step 1's report renderer.

**Risk the plan accepts on purpose:** Task 5 is a 6-12 hour wall-time task. If it crashes 80% through (Gemma-server OOM, power blip, filesystem full), there's no built-in resume — you'd re-run from scratch. Mitigations the plan already includes: (1) Step 3 spot-check after the warmup catches most bugs cheaply, (2) warmup pass in Step 2 validates the full task set before the 3-seed investment, (3) every run writes to `microbench.jsonl` incrementally so partial progress is recoverable by hand. A full resume mechanism is out of scope here; runaway-cost risk is zero (local, free), so the worst case is "restart tomorrow".

---

## Execution

Per project memory ("Subagent-Driven → always"), dispatch fresh subagents per task via `superpowers:subagent-driven-development`. Task 5 is long-running and best executed as a single background job the user can start in the evening and check the next morning; other tasks are short (≤30 min each).
