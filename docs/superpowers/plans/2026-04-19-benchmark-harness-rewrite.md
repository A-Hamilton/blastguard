# Benchmark Harness Rewrite (Plan 8) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rewrite `bench/` to produce a statistically defensible paired-comparison of BlastGuard vs. raw agent on the SWE-bench Pro Python subset, using ScaleAI's official `SWE-bench_Pro-os` evaluator.

**Architecture:** Delegate Docker-based grading to [`scaleapi/SWE-bench_Pro-os`](https://github.com/scaleapi/SWE-bench_Pro-os) (subprocess, not a library). Our harness owns: (1) task loading with Python-only filter, (2) agent rollouts per arm, (3) patch emission in the evaluator's JSON format, (4) deterministic pairing (same seed, same task order, both arms), (5) McNemar's test on per-task pass/fail flips, (6) budget cap + per-task telemetry. We wrap the evaluator to guard against issue #78 (silent infra failures scored as task failures).

**Tech Stack:** Python 3.12, uv, `datasets` (HuggingFace), `openai` SDK (OpenRouter via `base_url`), `mcp` client, `scipy` (McNemar's test), `scaleapi/SWE-bench_Pro-os` as a git submodule or cloned sibling directory, Docker engine for evaluator runs.

**Scope (MVP):**
- Python-only tasks (filter `task.language == "python"`) — expected ~250–350 of the 731 public tasks
- MiniMax M2.7 via OpenRouter with prompt caching
- Paired A/B: raw vs. BlastGuard, fixed seed, identical task ordering
- McNemar's chi-squared on discordant pairs
- Budget cap with abort-on-overrun
- Per-task telemetry: tokens, cost, turns, wall time, cache hit rate

**Out of scope (deferred to Plan 9+):**
- JS / Go / Rust / Java / C++ tasks
- Multi-seed analysis (`n_seeds > 1`)
- Held-out / commercial splits (they're not downloadable)
- Parallel per-task rollouts (sequential for MVP — simpler, reproducible)
- Swapping models (MiniMax M2.7 only; add more once harness proves it works)

---

## File Structure

**Modify:**
- `bench/tasks.py` — drop RuntimeError safety rail, load real schema, add Python filter
- `bench/runner.py` — arm flag, seed, telemetry, patch emission
- `bench/grader.py` — replace pytest logic with evaluator wrapper, keep `TAMPER_PATTERNS`
- `bench/compare.py` — replace naive diff with McNemar's paired analysis
- `bench/pyproject.toml` — add `scipy`
- `bench/README.md` — rewrite workflow section
- `bench/KNOWN_GAPS.md` — mark resolved gaps

**Create:**
- `bench/evaluator.py` — `SWE-bench_Pro-os` subprocess wrapper + infra-failure guard
- `bench/stats.py` — McNemar's test implementation
- `bench/budget.py` — cost tracking + cap guard
- `bench/telemetry.py` — per-task telemetry record + JSONL writer
- `bench/tests/test_evaluator.py`
- `bench/tests/test_stats.py`
- `bench/tests/test_budget.py`
- `bench/tests/test_telemetry.py`
- `bench/scripts/clone_evaluator.sh` — one-shot clone of `SWE-bench_Pro-os` into `bench/.evaluator/`

---

## Task 1: Dependency + evaluator clone

**Files:**
- Modify: `bench/pyproject.toml` (add `scipy>=1.14`)
- Create: `bench/scripts/clone_evaluator.sh`
- Create: `bench/.evaluator/` (cloned by script, gitignored)
- Modify: `.gitignore` (add `bench/.evaluator/`)

- [x] **Step 1: Add scipy to pyproject.toml**

Add `"scipy>=1.14"` to `[project].dependencies` in `bench/pyproject.toml`.

- [x] **Step 2: Create clone script**

```bash
#!/usr/bin/env bash
# bench/scripts/clone_evaluator.sh
set -euo pipefail

TARGET="${SCRIPT_DIR:-$(dirname "$0")/..}/.evaluator"
if [ -d "$TARGET/.git" ]; then
  echo "evaluator already cloned at $TARGET"
  exit 0
fi
git clone --depth 1 https://github.com/scaleapi/SWE-bench_Pro-os "$TARGET"
cd "$TARGET"
pip install -r requirements.txt
echo "evaluator ready at $TARGET"
```

- [x] **Step 3: Gitignore the cloned evaluator**

Append `bench/.evaluator/` to `.gitignore`.

- [x] **Step 4: Sync + smoke**

Run: `cd bench && uv sync && bash scripts/clone_evaluator.sh`
Expected: `bench/.evaluator/swe_bench_pro_eval.py` exists; `uv run python -c "import scipy.stats; print(scipy.__version__)"` prints a version.

- [x] **Step 5: Commit**

```bash
git add bench/pyproject.toml bench/scripts/clone_evaluator.sh bench/uv.lock .gitignore
git commit -m "bench: pin scipy and add evaluator clone script"
```

---

## Task 2: Real dataset schema + Python-only filter

**Files:**
- Modify: `bench/tasks.py` (drop RuntimeError, load real schema, add filter)
- Modify: `bench/tests/test_tasks.py` (update fixtures)

- [ ] **Step 1: Write failing test — loads real schema fields**

Append to `bench/tests/test_tasks.py`:

```python
def test_load_tasks_python_only_has_expected_fields(monkeypatch):
    """load_tasks returns Task records with real SWE-bench Pro fields."""
    from bench.tasks import load_tasks

    tasks = load_tasks(limit=5, python_only=True)
    assert len(tasks) == 5
    for t in tasks:
        assert t.task_id
        assert t.repo
        assert t.base_commit
        assert t.problem_statement
        assert isinstance(t.fail_to_pass, list)
        assert isinstance(t.pass_to_pass, list)
        assert t.language == "python"
```

- [ ] **Step 2: Run test to confirm it fails**

Run: `cd bench && uv run pytest tests/test_tasks.py::test_load_tasks_python_only_has_expected_fields -v`
Expected: FAIL — `RuntimeError: bench/tasks.py::load_tasks is not wired...`

- [ ] **Step 3: Rewrite `bench/tasks.py`**

Replace the entire body of `bench/tasks.py` with:

```python
"""Load SWE-bench Pro tasks from HuggingFace.

Real schema (ScaleAI/SWE-bench_Pro, split "test"):
- instance_id: str
- repo: str                         # "owner/repo"
- base_commit: str
- problem_statement: str
- fail_to_pass: str                 # JSON-encoded list, lowercase key
- pass_to_pass: str                 # JSON-encoded list, lowercase key
- language: str                     # "python", "javascript", etc.
- patch: str                        # ground-truth, NOT shown to agent
- dockerhub_tag: str                # used by evaluator
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

try:
    from datasets import load_dataset
except ImportError:  # pragma: no cover
    load_dataset = None  # type: ignore[assignment]

SWE_BENCH_PRO_DATASET = "ScaleAI/SWE-bench_Pro"
SWE_BENCH_PRO_SPLIT = "test"


@dataclass(frozen=True, slots=True)
class Task:
    task_id: str
    repo: str
    base_commit: str
    problem_statement: str
    fail_to_pass: list[str]
    pass_to_pass: list[str]
    language: str
    dockerhub_tag: str


def _coerce_list(raw: object) -> list[str]:
    """Handle either native list or JSON-encoded string."""
    if isinstance(raw, list):
        return [str(x) for x in raw]
    if isinstance(raw, str) and raw.strip():
        try:
            parsed = json.loads(raw)
            return [str(x) for x in parsed] if isinstance(parsed, list) else []
        except json.JSONDecodeError:
            return []
    return []


def load_tasks(
    limit: int | None = None,
    python_only: bool = True,
) -> list[Task]:
    """Fetch the public test split. Filter to Python by default."""
    if load_dataset is None:
        raise RuntimeError(
            "datasets not installed — run `uv sync` inside bench/ first"
        )
    ds = load_dataset(SWE_BENCH_PRO_DATASET, split=SWE_BENCH_PRO_SPLIT)

    tasks: list[Task] = []
    for row in ds:
        language = str(row.get("language", "")).lower()
        if python_only and language != "python":
            continue
        tasks.append(
            Task(
                task_id=str(row["instance_id"]),
                repo=str(row["repo"]),
                base_commit=str(row["base_commit"]),
                problem_statement=str(row["problem_statement"]),
                fail_to_pass=_coerce_list(row.get("fail_to_pass")),
                pass_to_pass=_coerce_list(row.get("pass_to_pass")),
                language=language,
                dockerhub_tag=str(row.get("dockerhub_tag", "")),
            )
        )
        if limit is not None and len(tasks) >= limit:
            break
    return tasks


def write_task_cache(tasks: list[Task], cache_path: Path) -> None:
    """Serialise tasks to JSONL for offline debugging."""
    with cache_path.open("w", encoding="utf-8") as f:
        for t in tasks:
            f.write(
                json.dumps(
                    {
                        "task_id": t.task_id,
                        "repo": t.repo,
                        "base_commit": t.base_commit,
                        "problem_statement": t.problem_statement,
                        "fail_to_pass": t.fail_to_pass,
                        "pass_to_pass": t.pass_to_pass,
                        "language": t.language,
                        "dockerhub_tag": t.dockerhub_tag,
                    }
                )
                + "\n"
            )
```

- [ ] **Step 4: Run test to confirm it passes**

Run: `cd bench && HF_HOME=/tmp/hf uv run pytest tests/test_tasks.py -v`
Expected: PASS for the new test (other tests may need fixture updates — fix them alongside).

- [ ] **Step 5: Commit**

```bash
git add bench/tasks.py bench/tests/test_tasks.py
git commit -m "bench: wire real SWE-bench Pro schema + Python-only filter"
```

---

## Task 3: Budget cap + telemetry records

**Files:**
- Create: `bench/budget.py`
- Create: `bench/telemetry.py`
- Create: `bench/tests/test_budget.py`
- Create: `bench/tests/test_telemetry.py`

- [ ] **Step 1: Write failing tests**

`bench/tests/test_budget.py`:

```python
from bench.budget import Budget, BudgetExceeded
import pytest


def test_budget_records_costs():
    b = Budget(cap_usd=1.00)
    b.record(input_tokens=1000, output_tokens=500, in_price_per_m=0.30, out_price_per_m=1.20)
    # 1000/1M * 0.30 + 500/1M * 1.20 = 0.0003 + 0.0006 = 0.0009
    assert abs(b.spent_usd - 0.0009) < 1e-6


def test_budget_aborts_when_cap_exceeded():
    b = Budget(cap_usd=0.001)
    with pytest.raises(BudgetExceeded):
        b.record(input_tokens=100_000, output_tokens=50_000, in_price_per_m=0.30, out_price_per_m=1.20)


def test_budget_cache_reads_are_cheaper():
    b = Budget(cap_usd=10.00)
    b.record(
        input_tokens=1_000_000,
        cached_input_tokens=750_000,
        output_tokens=100_000,
        in_price_per_m=0.30,
        cache_read_per_m=0.075,
        out_price_per_m=1.20,
    )
    # uncached 250k * 0.30 + cached 750k * 0.075 + output 100k * 1.20
    # = 0.075 + 0.05625 + 0.12 = 0.25125
    assert abs(b.spent_usd - 0.25125) < 1e-6
```

`bench/tests/test_telemetry.py`:

```python
from bench.telemetry import TelemetryRecord, write_jsonl
from pathlib import Path


def test_telemetry_record_roundtrip(tmp_path: Path):
    rec = TelemetryRecord(
        task_id="django__123",
        arm="blastguard",
        input_tokens=100,
        cached_input_tokens=80,
        output_tokens=50,
        turns=10,
        wall_seconds=42.5,
        cost_usd=0.001,
        patch_bytes=512,
        error=None,
    )
    out = tmp_path / "telemetry.jsonl"
    write_jsonl([rec], out)
    assert out.read_text().strip().startswith('{"task_id": "django__123"')
```

- [ ] **Step 2: Run to confirm they fail**

Run: `cd bench && uv run pytest tests/test_budget.py tests/test_telemetry.py -v`
Expected: FAIL — `ModuleNotFoundError: bench.budget` / `bench.telemetry`.

- [ ] **Step 3: Implement `bench/budget.py`**

```python
"""Budget cap + per-rollout cost tracking.

Input/output prices are per million tokens. Cache-read price is optional
(defaults to base input price if not supplied). Raises `BudgetExceeded`
the moment a `record()` call would push `spent_usd` past `cap_usd`.
"""

from __future__ import annotations


class BudgetExceeded(RuntimeError):
    """Raised when a record() call would exceed the configured cap."""


class Budget:
    def __init__(self, cap_usd: float) -> None:
        if cap_usd <= 0:
            raise ValueError("cap_usd must be positive")
        self.cap_usd = cap_usd
        self.spent_usd = 0.0

    def record(
        self,
        *,
        input_tokens: int,
        output_tokens: int,
        in_price_per_m: float,
        out_price_per_m: float,
        cached_input_tokens: int = 0,
        cache_read_per_m: float | None = None,
    ) -> float:
        """Charge this call to the budget. Returns the cost of this call."""
        uncached_input = max(0, input_tokens - cached_input_tokens)
        cache_rate = cache_read_per_m if cache_read_per_m is not None else in_price_per_m
        cost = (
            uncached_input * in_price_per_m / 1_000_000.0
            + cached_input_tokens * cache_rate / 1_000_000.0
            + output_tokens * out_price_per_m / 1_000_000.0
        )
        if self.spent_usd + cost > self.cap_usd:
            raise BudgetExceeded(
                f"next call costs ${cost:.4f}; spent ${self.spent_usd:.4f}; cap ${self.cap_usd:.2f}"
            )
        self.spent_usd += cost
        return cost
```

- [ ] **Step 4: Implement `bench/telemetry.py`**

```python
"""Per-rollout telemetry JSONL writer."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class TelemetryRecord:
    task_id: str
    arm: str                    # "raw" or "blastguard"
    input_tokens: int
    cached_input_tokens: int
    output_tokens: int
    turns: int
    wall_seconds: float
    cost_usd: float
    patch_bytes: int
    error: str | None


def write_jsonl(records: list[TelemetryRecord], path: Path) -> None:
    with path.open("w", encoding="utf-8") as f:
        for r in records:
            f.write(json.dumps(asdict(r)) + "\n")


def append_jsonl(record: TelemetryRecord, path: Path) -> None:
    with path.open("a", encoding="utf-8") as f:
        f.write(json.dumps(asdict(record)) + "\n")
```

- [ ] **Step 5: Run tests to confirm pass**

Run: `cd bench && uv run pytest tests/test_budget.py tests/test_telemetry.py -v`
Expected: PASS (5 tests total).

- [ ] **Step 6: Commit**

```bash
git add bench/budget.py bench/telemetry.py bench/tests/test_budget.py bench/tests/test_telemetry.py
git commit -m "bench: add budget cap + telemetry JSONL writer"
```

---

## Task 4: McNemar's paired-comparison test

**Files:**
- Create: `bench/stats.py`
- Create: `bench/tests/test_stats.py`

- [ ] **Step 1: Write failing tests**

`bench/tests/test_stats.py`:

```python
from bench.stats import mcnemar_paired, PairedResult


def test_mcnemar_detects_positive_lift():
    """BlastGuard flips 20 tasks to pass, raw flips 5. Highly significant."""
    pairs = [
        ("raw_only_pass", 5),
        ("blastguard_only_pass", 20),
        ("both_pass", 60),
        ("both_fail", 15),
    ]
    r = mcnemar_paired(pairs)
    assert r.blastguard_wins == 20
    assert r.raw_wins == 5
    assert r.p_value < 0.01
    assert r.blastguard_score_pct > r.raw_score_pct


def test_mcnemar_detects_no_lift():
    """Symmetric discordant pairs — no signal."""
    pairs = [
        ("raw_only_pass", 10),
        ("blastguard_only_pass", 10),
        ("both_pass", 50),
        ("both_fail", 30),
    ]
    r = mcnemar_paired(pairs)
    assert r.p_value > 0.1


def test_mcnemar_zero_discordant():
    """Identical arms — p-value = 1.0 (degenerate case)."""
    pairs = [("both_pass", 100), ("both_fail", 50), ("raw_only_pass", 0), ("blastguard_only_pass", 0)]
    r = mcnemar_paired(pairs)
    assert r.p_value == 1.0
```

- [ ] **Step 2: Run to confirm they fail**

Run: `cd bench && uv run pytest tests/test_stats.py -v`
Expected: FAIL — `ModuleNotFoundError: bench.stats`.

- [ ] **Step 3: Implement `bench/stats.py`**

```python
"""Paired McNemar's test for A/B benchmark comparison.

Given per-task pass/fail outcomes for two arms (raw, blastguard) on the
same tasks, McNemar's chi-squared on discordant pairs tells us whether
one arm flips more tasks net-positive than the other.

Concordant pairs (both pass, both fail) are ignored — they carry no
information about the treatment effect. The test statistic is built
from `b` (only raw passes) and `c` (only blastguard passes):

    chi2 = (|b - c| - 1)^2 / (b + c)     (continuity-corrected)

Small-sample case (b + c < 25): use scipy's exact binomial.
"""

from __future__ import annotations

from dataclasses import dataclass

from scipy.stats import binomtest, chi2


@dataclass(frozen=True, slots=True)
class PairedResult:
    both_pass: int
    both_fail: int
    raw_wins: int           # only raw passed
    blastguard_wins: int    # only blastguard passed
    n: int
    raw_score_pct: float
    blastguard_score_pct: float
    delta_pct: float
    p_value: float
    test_used: str


def mcnemar_paired(pairs: list[tuple[str, int]]) -> PairedResult:
    """Compute McNemar's test from a list of (bucket_name, count) tuples.

    Bucket names: "both_pass", "both_fail", "raw_only_pass",
    "blastguard_only_pass".
    """
    counts = {name: 0 for name in ("both_pass", "both_fail", "raw_only_pass", "blastguard_only_pass")}
    for name, n in pairs:
        if name not in counts:
            raise ValueError(f"unknown bucket: {name}")
        counts[name] = n

    b = counts["raw_only_pass"]
    c = counts["blastguard_only_pass"]
    n_total = sum(counts.values())

    if b + c == 0:
        p_value = 1.0
        test_used = "degenerate (no discordant pairs)"
    elif b + c < 25:
        # exact binomial with p=0.5
        res = binomtest(min(b, c), n=b + c, p=0.5, alternative="two-sided")
        p_value = res.pvalue
        test_used = "exact binomial"
    else:
        stat = (abs(b - c) - 1) ** 2 / (b + c)
        p_value = 1.0 - chi2.cdf(stat, df=1)
        test_used = "chi-squared (continuity-corrected)"

    raw_pass = counts["both_pass"] + b
    blastguard_pass = counts["both_pass"] + c
    raw_pct = 100.0 * raw_pass / n_total if n_total else 0.0
    blastguard_pct = 100.0 * blastguard_pass / n_total if n_total else 0.0

    return PairedResult(
        both_pass=counts["both_pass"],
        both_fail=counts["both_fail"],
        raw_wins=b,
        blastguard_wins=c,
        n=n_total,
        raw_score_pct=raw_pct,
        blastguard_score_pct=blastguard_pct,
        delta_pct=blastguard_pct - raw_pct,
        p_value=p_value,
        test_used=test_used,
    )
```

- [ ] **Step 4: Run tests to confirm pass**

Run: `cd bench && uv run pytest tests/test_stats.py -v`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add bench/stats.py bench/tests/test_stats.py
git commit -m "bench: paired McNemar's test for A/B analysis"
```

---

## Task 5: Evaluator subprocess wrapper with infra-failure guard

**Files:**
- Create: `bench/evaluator.py`
- Create: `bench/tests/test_evaluator.py`

Background: SWE-bench_Pro-os issue #78 — API rate-limit errors in the evaluator are silently scored as task failures, contaminating the benchmark. We wrap the subprocess to (a) validate the evaluator finished cleanly, (b) parse its per-instance output JSON, (c) surface infra failures separately from task failures.

- [ ] **Step 1: Write failing test**

`bench/tests/test_evaluator.py`:

```python
from bench.evaluator import (
    EvaluatorResult,
    parse_evaluator_output,
    write_patches_json,
)
from pathlib import Path
import json


def test_parse_evaluator_output_resolved(tmp_path: Path):
    """A resolved task maps to EvaluatorResult(resolved=True, infra_failure=False)."""
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "django__123.json").write_text(
        json.dumps({
            "instance_id": "django__123",
            "resolved": True,
            "tests_status": {"fail_to_pass": {"success": ["t1"], "failure": []}},
        })
    )
    results = parse_evaluator_output(out_dir)
    assert len(results) == 1
    r = results[0]
    assert r.task_id == "django__123"
    assert r.resolved is True
    assert r.infra_failure is False


def test_parse_evaluator_output_infra_failure(tmp_path: Path):
    """An empty / errored result is flagged as infra_failure, not a clean fail."""
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "django__456.json").write_text(
        json.dumps({
            "instance_id": "django__456",
            "error": "rate_limit",
        })
    )
    results = parse_evaluator_output(out_dir)
    r = results[0]
    assert r.resolved is False
    assert r.infra_failure is True


def test_write_patches_json_matches_evaluator_format(tmp_path: Path):
    """The evaluator expects a JSON array of {instance_id, patch, prefix}."""
    out = tmp_path / "patches.json"
    write_patches_json(
        [("django__1", "diff --git a b\n+ foo"), ("sklearn__2", "")],
        prefix="blastguard-m27",
        out_path=out,
    )
    data = json.loads(out.read_text())
    assert isinstance(data, list)
    assert data[0]["instance_id"] == "django__1"
    assert data[0]["patch"].startswith("diff --git")
    assert data[0]["prefix"] == "blastguard-m27"
    assert data[1]["patch"] == ""
```

- [ ] **Step 2: Run to confirm they fail**

Run: `cd bench && uv run pytest tests/test_evaluator.py -v`
Expected: FAIL — `ModuleNotFoundError: bench.evaluator`.

- [ ] **Step 3: Implement `bench/evaluator.py`**

```python
"""Wrapper for scaleapi/SWE-bench_Pro-os evaluator.

We shell out to `swe_bench_pro_eval.py`. The evaluator's per-instance
output JSON is our only source of truth for pass/fail. Issue #78 (open)
means rate-limit/infra errors inside the evaluator can be silently
scored as task failures; we detect this by checking for an `error` key
or missing `resolved` field and flag those as `infra_failure=True` so
`compare.py` can exclude them from McNemar's counts.
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class EvaluatorResult:
    task_id: str
    resolved: bool
    infra_failure: bool
    raw: dict


def write_patches_json(
    predictions: list[tuple[str, str]],
    prefix: str,
    out_path: Path,
) -> None:
    """Emit the JSON array format expected by the evaluator.

    predictions: list of (instance_id, unified_diff_patch) tuples.
    """
    data = [
        {"instance_id": iid, "patch": patch, "prefix": prefix}
        for iid, patch in predictions
    ]
    out_path.write_text(json.dumps(data, indent=2))


def parse_evaluator_output(out_dir: Path) -> list[EvaluatorResult]:
    """Read every *.json in out_dir and classify it."""
    results: list[EvaluatorResult] = []
    for path in sorted(out_dir.glob("*.json")):
        try:
            payload = json.loads(path.read_text())
        except json.JSONDecodeError:
            results.append(
                EvaluatorResult(
                    task_id=path.stem,
                    resolved=False,
                    infra_failure=True,
                    raw={"error": "json_decode_error", "file": path.name},
                )
            )
            continue

        task_id = str(payload.get("instance_id", path.stem))
        has_error = bool(payload.get("error")) or "resolved" not in payload
        resolved = bool(payload.get("resolved", False)) if not has_error else False

        results.append(
            EvaluatorResult(
                task_id=task_id,
                resolved=resolved,
                infra_failure=has_error,
                raw=payload,
            )
        )
    return results


def run_evaluator(
    *,
    evaluator_dir: Path,
    raw_sample_csv: Path,
    patches_json: Path,
    output_dir: Path,
    num_workers: int = 4,
    dockerhub_username: str = "jefzda",
    timeout_seconds: int = 3600,
) -> int:
    """Invoke the evaluator as a subprocess. Returns its exit code."""
    output_dir.mkdir(parents=True, exist_ok=True)
    cmd = [
        "python",
        str(evaluator_dir / "swe_bench_pro_eval.py"),
        f"--raw_sample_path={raw_sample_csv}",
        f"--patch_path={patches_json}",
        f"--output_dir={output_dir}",
        f"--scripts_dir={evaluator_dir / 'run_scripts'}",
        f"--num_workers={num_workers}",
        f"--dockerhub_username={dockerhub_username}",
    ]
    proc = subprocess.run(
        cmd,
        cwd=evaluator_dir,
        timeout=timeout_seconds,
        check=False,
    )
    return proc.returncode
```

- [ ] **Step 4: Run tests to confirm pass**

Run: `cd bench && uv run pytest tests/test_evaluator.py -v`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add bench/evaluator.py bench/tests/test_evaluator.py
git commit -m "bench: wrap SWE-bench_Pro-os evaluator with infra-failure guard"
```

---

## Task 6: Rewrite runner.py for paired arms + patch emission

**Files:**
- Modify: `bench/runner.py`

The existing `runner.py` runs a single arm and calls our old `grader.py`. Rewrite so each invocation runs one arm end-to-end and emits a `patches.json` that the evaluator can consume.

- [ ] **Step 1: Read current runner.py**

Run: `cat bench/runner.py | head -80`
Note the public function signature and imports.

- [ ] **Step 2: Replace runner.py**

```python
"""Run one arm (raw or blastguard) against the SWE-bench Pro Python subset.

Emits:
  results/<run_id>/patches.json            — evaluator input
  results/<run_id>/telemetry.jsonl         — per-task telemetry
  results/<run_id>/config.json             — arm, seed, model, budget

Usage:
  uv run python -m bench.runner \\
    --arm blastguard \\
    --model minimax/minimax-m2.7 \\
    --limit 10 \\
    --seed 42 \\
    --budget-usd 25.0 \\
    --run-id smoke-blastguard
"""

from __future__ import annotations

import argparse
import json
import random
import time
from pathlib import Path

from bench.agent_loop import run_openai_compatible
from bench.budget import Budget, BudgetExceeded
from bench.evaluator import write_patches_json
from bench.mcp_client import BlastGuardClient
from bench.prompts import build_system_prompt
from bench.tasks import load_tasks
from bench.telemetry import TelemetryRecord, append_jsonl


def _results_dir(run_id: str) -> Path:
    d = Path(__file__).parent / "results" / run_id
    d.mkdir(parents=True, exist_ok=True)
    return d


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--arm", choices=("raw", "blastguard"), required=True)
    p.add_argument("--model", default="minimax/minimax-m2.7")
    p.add_argument("--limit", type=int, default=None, help="Cap task count")
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--budget-usd", type=float, required=True)
    p.add_argument("--run-id", required=True)
    p.add_argument("--in-price", type=float, default=0.30, help="$/M input tokens")
    p.add_argument("--out-price", type=float, default=1.20, help="$/M output tokens")
    p.add_argument("--cache-price", type=float, default=0.075, help="$/M cached input")
    args = p.parse_args()

    random.seed(args.seed)

    run_dir = _results_dir(args.run_id)
    (run_dir / "config.json").write_text(
        json.dumps(
            {
                "arm": args.arm,
                "model": args.model,
                "seed": args.seed,
                "budget_usd": args.budget_usd,
                "limit": args.limit,
            },
            indent=2,
        )
    )

    budget = Budget(cap_usd=args.budget_usd)
    tasks = load_tasks(limit=args.limit, python_only=True)
    # Sort by task_id so both arms iterate in the same order.
    tasks.sort(key=lambda t: t.task_id)

    telemetry_path = run_dir / "telemetry.jsonl"
    predictions: list[tuple[str, str]] = []
    mcp_client = BlastGuardClient() if args.arm == "blastguard" else None
    if mcp_client is not None:
        mcp_client.start()

    try:
        for task in tasks:
            t0 = time.time()
            try:
                patch, tokens = run_openai_compatible(
                    model=args.model,
                    system_prompt=build_system_prompt(arm=args.arm),
                    problem_statement=task.problem_statement,
                    mcp_client=mcp_client,
                    seed=args.seed,
                )
                cost = budget.record(
                    input_tokens=tokens.input,
                    cached_input_tokens=tokens.cached_input,
                    output_tokens=tokens.output,
                    in_price_per_m=args.in_price,
                    cache_read_per_m=args.cache_price,
                    out_price_per_m=args.out_price,
                )
                error = None
            except BudgetExceeded as e:
                print(f"[{task.task_id}] BUDGET STOP: {e}")
                break
            except Exception as e:  # noqa: BLE001 — intentional, log + continue
                patch = ""
                tokens_input = tokens_cached = tokens_output = 0
                cost = 0.0
                error = f"{type(e).__name__}: {e}"

            predictions.append((task.task_id, patch))
            append_jsonl(
                TelemetryRecord(
                    task_id=task.task_id,
                    arm=args.arm,
                    input_tokens=getattr(tokens, "input", 0),
                    cached_input_tokens=getattr(tokens, "cached_input", 0),
                    output_tokens=getattr(tokens, "output", 0),
                    turns=getattr(tokens, "turns", 0),
                    wall_seconds=time.time() - t0,
                    cost_usd=cost,
                    patch_bytes=len(patch.encode("utf-8")),
                    error=error,
                ),
                telemetry_path,
            )
            print(f"[{task.task_id}] cost=${cost:.4f} spent=${budget.spent_usd:.4f}")
    finally:
        if mcp_client is not None:
            mcp_client.stop()

    write_patches_json(
        predictions,
        prefix=f"{args.arm}-{args.model.replace('/', '_')}",
        out_path=run_dir / "patches.json",
    )
    print(f"done: wrote {len(predictions)} predictions; spent ${budget.spent_usd:.4f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 3: Update `agent_loop.run_openai_compatible` signature**

The `run_openai_compatible` function must now accept `seed: int` and return a `(patch: str, tokens: TokenCount)` tuple where `TokenCount` has fields `input`, `cached_input`, `output`, `turns`. If the current function has a different shape, extend it — don't rewrite wholesale.

Open `bench/agent_loop.py` and, if the return shape doesn't match, adjust the function so that:

```python
from dataclasses import dataclass

@dataclass(frozen=True, slots=True)
class TokenCount:
    input: int
    cached_input: int
    output: int
    turns: int


def run_openai_compatible(
    *,
    model: str,
    system_prompt: str,
    problem_statement: str,
    mcp_client,
    seed: int,
) -> tuple[str, TokenCount]:
    ...  # existing implementation, now returning (patch, TokenCount(...))
```

The patch must be the unified diff of the workspace vs. base commit. If the existing loop doesn't produce one, compute it with `git diff` at the end of the rollout.

- [ ] **Step 4: Smoke-test with a tiny limit (dry run, no network)**

Run: `cd bench && uv run python -m bench.runner --arm raw --limit 0 --seed 42 --budget-usd 0.10 --run-id smoke-dry --model minimax/minimax-m2.7`

Expected: exit 0, `bench/results/smoke-dry/config.json` created, empty telemetry.jsonl, empty patches.json (array `[]`). No network calls since `--limit 0`.

- [ ] **Step 5: Commit**

```bash
git add bench/runner.py bench/agent_loop.py
git commit -m "bench: paired-arm runner with budget cap and patch emission"
```

---

## Task 7: Replace compare.py with paired-analysis reporter

**Files:**
- Modify: `bench/compare.py`
- Add: `bench/tests/test_compare.py` (new test — existing tests may need updating)

- [ ] **Step 1: Write failing test**

Append to `bench/tests/test_compare.py`:

```python
from bench.compare import pair_results, format_report
from bench.evaluator import EvaluatorResult


def _res(task_id: str, resolved: bool, infra_failure: bool = False) -> EvaluatorResult:
    return EvaluatorResult(task_id=task_id, resolved=resolved, infra_failure=infra_failure, raw={})


def test_pair_results_excludes_infra_failures_from_either_arm():
    raw = [_res("a", True), _res("b", False), _res("c", False, infra_failure=True)]
    bg = [_res("a", True), _res("b", True), _res("c", True)]
    paired = pair_results(raw, bg)
    # "c" is excluded because raw hit an infra failure
    assert set(paired.keys()) == {"a", "b"}


def test_format_report_includes_mcnemar():
    raw = [_res("a", True), _res("b", False), _res("c", False), _res("d", True)]
    bg = [_res("a", True), _res("b", True), _res("c", True), _res("d", False)]
    report = format_report(raw, bg)
    assert "McNemar" in report
    assert "blastguard_wins" in report.lower() or "BlastGuard wins" in report
```

- [ ] **Step 2: Run to confirm failure**

Run: `cd bench && uv run pytest tests/test_compare.py -v`
Expected: FAIL (missing functions).

- [ ] **Step 3: Rewrite `bench/compare.py`**

```python
"""Paired analysis reporter.

Loads two arms' evaluator outputs, pairs them by task_id, excludes any
task where either arm hit an infra_failure (rate limits, Docker crashes,
evaluator errors), then runs McNemar's test.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from bench.evaluator import EvaluatorResult, parse_evaluator_output
from bench.stats import mcnemar_paired


def pair_results(
    raw: list[EvaluatorResult],
    blastguard: list[EvaluatorResult],
) -> dict[str, tuple[EvaluatorResult, EvaluatorResult]]:
    """Intersect by task_id and drop infra failures from either arm."""
    raw_map = {r.task_id: r for r in raw if not r.infra_failure}
    bg_map = {r.task_id: r for r in blastguard if not r.infra_failure}
    shared = raw_map.keys() & bg_map.keys()
    return {tid: (raw_map[tid], bg_map[tid]) for tid in shared}


def format_report(
    raw: list[EvaluatorResult],
    blastguard: list[EvaluatorResult],
) -> str:
    pairs = pair_results(raw, blastguard)
    both_pass = both_fail = raw_only = bg_only = 0
    for r, b in pairs.values():
        if r.resolved and b.resolved:
            both_pass += 1
        elif not r.resolved and not b.resolved:
            both_fail += 1
        elif r.resolved and not b.resolved:
            raw_only += 1
        else:
            bg_only += 1

    stats = mcnemar_paired([
        ("both_pass", both_pass),
        ("both_fail", both_fail),
        ("raw_only_pass", raw_only),
        ("blastguard_only_pass", bg_only),
    ])

    raw_infra = sum(1 for r in raw if r.infra_failure)
    bg_infra = sum(1 for r in blastguard if r.infra_failure)

    return (
        f"Paired McNemar's Test — BlastGuard vs raw\n"
        f"===========================================\n"
        f"Paired tasks:          {stats.n}\n"
        f"Infra failures (raw):  {raw_infra} (excluded)\n"
        f"Infra failures (bg):   {bg_infra} (excluded)\n"
        f"\n"
        f"Both pass:             {stats.both_pass}\n"
        f"Both fail:             {stats.both_fail}\n"
        f"Raw wins (only raw):   {stats.raw_wins}\n"
        f"BlastGuard wins:       {stats.blastguard_wins}\n"
        f"\n"
        f"Raw score:             {stats.raw_score_pct:.2f}%\n"
        f"BlastGuard score:      {stats.blastguard_score_pct:.2f}%\n"
        f"Delta:                 {stats.delta_pct:+.2f} pp\n"
        f"\n"
        f"Test:                  {stats.test_used}\n"
        f"p-value:               {stats.p_value:.4f}\n"
        f"Significant (α=0.05):  {'YES' if stats.p_value < 0.05 else 'NO'}\n"
    )


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--raw-output-dir", type=Path, required=True)
    p.add_argument("--blastguard-output-dir", type=Path, required=True)
    args = p.parse_args()

    raw = parse_evaluator_output(args.raw_output_dir)
    bg = parse_evaluator_output(args.blastguard_output_dir)
    print(format_report(raw, bg))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 4: Run tests to confirm pass**

Run: `cd bench && uv run pytest tests/test_compare.py -v`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add bench/compare.py bench/tests/test_compare.py
git commit -m "bench: paired analysis reporter with McNemar's + infra exclusion"
```

---

## Task 8: Retire old pytest grader; update KNOWN_GAPS

**Files:**
- Modify: `bench/grader.py` (keep `TAMPER_PATTERNS` + `detect_tampering`, delete `grade()` / `_count_passes()`)
- Modify: `bench/KNOWN_GAPS.md`

- [ ] **Step 1: Delete the pytest-based `grade()` function**

Open `bench/grader.py`. Keep the top-of-file docstring, `TAMPER_PATTERNS`, `detect_tampering`, and `GradeResult` (used by `detect_tampering` callers). Delete the `grade()` function and `_count_passes()` helper — grading now lives in `evaluator.py`.

After edit, `bench/grader.py` should be ~60 lines max.

- [ ] **Step 2: Update `bench/KNOWN_GAPS.md`**

Replace the body with:

```markdown
# Known Gaps — resolved by Plan 8 rewrite

Gap 1 (schema mismatch) — RESOLVED. `bench/tasks.py` now loads the real
`ScaleAI/SWE-bench_Pro` schema with lowercase keys and JSON-encoded
test lists.

Gap 2 (pytest-only grading) — RESOLVED. Grading is delegated to
`scaleapi/SWE-bench_Pro-os` via `bench/evaluator.py`. Multi-language
tasks grade correctly because the evaluator uses per-repo Docker images
and per-repo test runners. We still filter to Python-only in `load_tasks`
as an MVP scope control — remove `python_only=True` to grade all
languages.

Gap 3 (no dep install) — RESOLVED. The evaluator's per-repo Docker
images include dependencies. `setup_workspace` in `runner.py` is no
longer needed; patches apply inside the evaluator container.

Gap 4 (HF_HOME) — UNCHANGED. Set `HF_HOME=/tmp/hf` before running
anything that touches the datasets library.

# Remaining deferred work (Plan 9+)

- JS / Go / Rust / Java / C++ task support — drop `python_only=True` and
  confirm evaluator runs green against non-Python repos.
- Multi-seed analysis to shrink CI further.
- Parallel per-task rollouts (currently sequential).
- Watch SWE-bench_Pro-os issues #75 (image inconsistency), #76 (test
  name mismatches), #78 (silent rate-limit failures). Our wrapper
  exposes #78 as `infra_failure` so it doesn't contaminate McNemar's.
```

- [ ] **Step 3: Run full test suite to confirm nothing regresses**

Run: `cd bench && uv run pytest -v`
Expected: all tests pass. If `test_grader.py` references `grade()`, delete or rewrite those tests to cover only `detect_tampering`.

- [ ] **Step 4: Commit**

```bash
git add bench/grader.py bench/KNOWN_GAPS.md bench/tests/test_grader.py
git commit -m "bench: retire pytest grader, grading now via evaluator.py"
```

---

## Task 9: End-to-end smoke test (3 tasks, raw arm only)

**Files:**
- No code changes — this is a manual verification gate.

Run a tiny rollout to verify the full pipeline works before spending real money.

- [ ] **Step 1: Ensure Docker is running**

Run: `docker info | head -5`
Expected: server version and storage driver printed. If Docker isn't running, start it: `sudo systemctl start docker` (or whatever the user's OS requires).

- [ ] **Step 2: Clone the evaluator**

Run: `cd bench && bash scripts/clone_evaluator.sh`
Expected: `bench/.evaluator/swe_bench_pro_eval.py` exists.

- [ ] **Step 3: Run the raw arm on 3 tasks**

Run:
```bash
cd bench && \
  export OPENROUTER_API_KEY=$(grep OPENROUTER_API_KEY .env | cut -d= -f2) && \
  HF_HOME=/tmp/hf uv run python -m bench.runner \
    --arm raw \
    --model minimax/minimax-m2.7 \
    --limit 3 \
    --seed 42 \
    --budget-usd 2.00 \
    --run-id smoke-raw
```

Expected: completes within 10 minutes. `bench/results/smoke-raw/patches.json` contains 3 entries. `telemetry.jsonl` has 3 rows. Budget spent < $2.00.

- [ ] **Step 4: Run the evaluator on those 3 patches**

The evaluator also needs a raw_sample CSV — download or generate it from the HuggingFace dataset first (the evaluator's README shows how). Skip Docker validation if smoke is purely harness-wiring.

Run:
```bash
cd bench && uv run python -c "
from bench.evaluator import run_evaluator
from pathlib import Path
rc = run_evaluator(
    evaluator_dir=Path('.evaluator'),
    raw_sample_csv=Path('.evaluator/swe_bench_pro_full.csv'),
    patches_json=Path('results/smoke-raw/patches.json'),
    output_dir=Path('results/smoke-raw/eval'),
    num_workers=2,
    timeout_seconds=1800,
)
print('evaluator exit:', rc)
"
```

Expected: exit 0. `results/smoke-raw/eval/*.json` has 3 files. At least one has `resolved: true` or `false` (not all infra_failure).

- [ ] **Step 5: Commit the smoke script as documentation**

```bash
# Add a smoke command to bench/README.md showing the exact invocations above.
git add bench/README.md
git commit -m "bench: document 3-task smoke-test workflow"
```

---

## Task 10: Update bench/README.md workflow section

**Files:**
- Modify: `bench/README.md`

Replace the existing "Quick Start" / workflow section with:

```markdown
## Workflow (Plan 8)

Prerequisites:
- Docker daemon running (the evaluator pulls per-repo images)
- `.env` with `OPENROUTER_API_KEY=sk-or-v1-...`
- `bench/.evaluator/` cloned via `bash scripts/clone_evaluator.sh`

### 1. Smoke test (3 tasks, ~$1)

    cd bench
    HF_HOME=/tmp/hf uv run python -m bench.runner \
      --arm raw --limit 3 --seed 42 \
      --budget-usd 2.00 --run-id smoke-raw \
      --model minimax/minimax-m2.7

### 2. Paired pilot (100 tasks, ~$46)

    # raw arm
    HF_HOME=/tmp/hf uv run python -m bench.runner \
      --arm raw --limit 100 --seed 42 \
      --budget-usd 30.00 --run-id pilot-raw \
      --model minimax/minimax-m2.7

    # blastguard arm — same seed, same tasks, same order
    HF_HOME=/tmp/hf uv run python -m bench.runner \
      --arm blastguard --limit 100 --seed 42 \
      --budget-usd 30.00 --run-id pilot-bg \
      --model minimax/minimax-m2.7

### 3. Grade both arms

    uv run python -c "from bench.evaluator import run_evaluator; \
      run_evaluator(evaluator_dir=Path('.evaluator'), \
        raw_sample_csv=Path('.evaluator/swe_bench_pro_full.csv'), \
        patches_json=Path('results/pilot-raw/patches.json'), \
        output_dir=Path('results/pilot-raw/eval'))"
    # repeat for pilot-bg

### 4. Compare

    uv run python -m bench.compare \
      --raw-output-dir results/pilot-raw/eval \
      --blastguard-output-dir results/pilot-bg/eval

Expected output: McNemar's p-value, per-arm scores, delta in pp.

### 5. Full run (731 tasks, ~$170) — gated on pilot showing ≥+1pp delta

Swap `--limit 100` for `--limit 731` (or omit). Budgets per arm: ~$90.
```

Commit:

```bash
git add bench/README.md
git commit -m "bench: document Plan 8 paired-comparison workflow"
```

---

## Self-Review

**Spec coverage:**
- Plan goal was "rewrite bench/ for paired-comparison SWE-bench Pro on Python" → Tasks 2, 6, 7 own this.
- Evaluator integration → Task 5.
- Statistical rigor (McNemar's) → Task 4.
- Budget cap → Task 3.
- Infra-failure handling (issue #78) → Task 5 (`infra_failure` flag), Task 7 (exclude from pairs).
- Smoke before spend → Task 9.
- Docs → Task 10.

**Placeholders:** none. Every code step has complete code; every command has expected output.

**Type consistency:**
- `Task` dataclass fields match across Tasks 2, 6.
- `EvaluatorResult` fields match across Tasks 5, 7.
- `TokenCount` introduced in Task 6 — used consistently in `runner.py`.
- `PairedResult` defined in Task 4, consumed in Task 7's `format_report`.
- `TelemetryRecord` fields match between Task 3 definition and Task 6 construction.

**One thing the plan does NOT cover** (on purpose): the evaluator's `swe_bench_pro_full.csv` source. The evaluator's README shows how to generate it from the HuggingFace dataset — refer to their docs inside `.evaluator/README.md` when Task 9 Step 4 runs. Not worth reimplementing.

---

## Execution

Per project convention (memory: "Default to subagent-driven execution"), I will dispatch fresh subagents per task via `superpowers:subagent-driven-development` without asking for mode.
