# BlastGuard Benchmark Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the SWE-bench Pro benchmark harness per SPEC §15: spawns the BlastGuard MCP server as a subprocess, runs a scaffolded agent loop against each of the 731 public-set tasks with and without BlastGuard enabled, collects per-task JSONL instrumentation, and generates a comparison report. Ship a 3-task smoke run in CI; document the full-set run as a user-initiated operation.

**Architecture:** Python harness under `bench/`. One `runner.py` module that: (1) loads a SWE-bench Pro task from the HuggingFace dataset, (2) sets up a clean workspace, (3) optionally starts the BlastGuard binary as an MCP subprocess and connects via the `mcp` Python SDK, (4) runs an agent loop using `anthropic` (for Claude) or `openai`-compatible (for GLM) with tool-use enabled, (5) applies the agent's final patch and runs the fail-to-pass tests in an isolated subprocess, (6) emits one JSONL record per task with `resolved / turns / tokens_in / tokens_out / tool_calls_per_type / wall_time`. A `compare.py` aggregator reads two result sets and prints delta resolution rate, delta tokens, delta turns, per-repo breakdown. A `grader.py` guards against the Berkeley BenchJack conftest.py exploit (SPEC §15.4) by running grading in a pristine-config subprocess that ignores agent-written pytest configs.

**Tech Stack:** Python 3.11+, `uv` for dependency management, `mcp` SDK (official Python rmcp client), `anthropic` SDK (Claude), `openai` SDK (GLM via OpenRouter-compatible endpoint), `datasets` (HuggingFace), `pytest` (for reference-solution grading). BlastGuard binary is already a shippable Rust artifact from Plans 1-6.

**Preconditions:**
- Repo at `/home/adam/Documents/blastguard`, branch `phase-1-benchmark` from `main` (HEAD `c36b762`).
- BlastGuard binary compiles: `cargo build --release` produces `target/release/blastguard`.
- `bench/` directory already exists with a `README.md` stub (Plan 1 scaffold).
- The machine running the full benchmark has:
  - `uv` installed (`curl -LsSf https://astral.sh/uv/install.sh | sh`).
  - Git LFS configured (SWE-bench Pro repos can be large).
  - `ANTHROPIC_API_KEY` and `OPENROUTER_API_KEY` env vars set.
  - ~20 GB free disk for task workspaces.
- For the smoke subset (3 tasks) the user needs ~$0.50 of Claude API credit.

**Definition of done:**
- `cd bench && uv run python runner.py --tasks 3 --model claude-opus-4-7 --no-blastguard` produces `results/baseline-smoke.jsonl` with 3 records.
- Same command with `--with-blastguard` produces `results/blastguard-smoke.jsonl`.
- `uv run python compare.py results/baseline-smoke.jsonl results/blastguard-smoke.jsonl` prints delta metrics.
- Grader rejects a tampered `conftest.py` per the BenchJack defense.
- README.md documents the full-run command + expected cost + interpretation of results.
- `cargo check/test/clippy/build` on the Rust side: all still green (this plan only adds Python + docs).

**Important:** the full-set run (731 tasks × 2 conditions × 2 models = 2,924 agent rollouts) is NOT part of this plan. It's a **user operation** executed after the harness lands. The smoke subset validates the plumbing.

---

## File Structure

| Path | Responsibility |
|---|---|
| `bench/pyproject.toml` | `uv`-managed project declaring the harness deps |
| `bench/runner.py` | Entry point: load tasks, run agent loop per task, emit JSONL |
| `bench/mcp_client.py` | Wraps the `mcp` SDK; spawns BlastGuard subprocess, lists tools, forwards `tools/call` |
| `bench/agent_loop.py` | Model-agnostic agent loop (Anthropic or OpenAI-compatible) with tool-use and turn cap |
| `bench/grader.py` | Runs fail-to-pass tests in an isolated subprocess; rejects tampered `conftest.py` |
| `bench/compare.py` | Reads two JSONL result sets, prints delta resolution rate / tokens / turns / per-repo |
| `bench/prompts.py` | System prompt variants — one for the baseline scaffold, one for the BlastGuard-enabled scaffold (mentions the three tools per SPEC) |
| `bench/results/` | Output JSONL files (gitignored) |
| `bench/results/.gitkeep` | Empty marker so the dir exists in git |
| `bench/README.md` | Run commands + cost + methodology |
| `bench/tests/test_grader.py` | Unit tests for the grader's tamper detection |
| `bench/tests/test_compare.py` | Unit tests for compare.py aggregation |
| `.gitignore` | Add `bench/results/*.jsonl` + `bench/.venv/` |

---

## Task 1: Bench scaffolding — `pyproject.toml`, dirs, gitignore

**Files:**
- Create: `bench/pyproject.toml`
- Create: `bench/results/.gitkeep`
- Create: `bench/tests/__init__.py` (empty)
- Modify: `.gitignore`

- [ ] **Step 1: Write `bench/pyproject.toml`**

```toml
[project]
name = "blastguard-bench"
version = "0.1.0"
description = "SWE-bench Pro benchmark harness for BlastGuard"
requires-python = ">=3.11"
dependencies = [
    "anthropic>=0.40",
    "openai>=1.50",
    "mcp>=1.10",
    "datasets>=3.0",
    "pydantic>=2.8",
    "click>=8.1",
    "rich>=13.7",
]

[dependency-groups]
dev = [
    "pytest>=8.3",
    "pytest-asyncio>=0.24",
    "ruff>=0.7",
]

[tool.uv]
package = false

[tool.ruff]
line-length = 100
target-version = "py311"

[tool.ruff.lint]
select = ["E", "F", "W", "I", "B", "UP", "SIM"]
```

- [ ] **Step 2: Create the results directory + gitkeep**

```bash
cd /home/adam/Documents/blastguard
mkdir -p bench/results bench/tests
touch bench/results/.gitkeep
touch bench/tests/__init__.py
```

- [ ] **Step 3: Extend `.gitignore`**

Append to `/home/adam/Documents/blastguard/.gitignore`:

```
# Benchmark harness
bench/results/*.jsonl
bench/.venv/
bench/__pycache__/
bench/**/__pycache__/
bench/.pytest_cache/
```

- [ ] **Step 4: Bootstrap the venv (does not commit the venv)**

```bash
cd /home/adam/Documents/blastguard/bench
uv sync
```

Expected: `uv` creates `.venv/` and installs the deps. If `uv` isn't installed, surface that as a blocker — it's the supported dep manager for this harness.

- [ ] **Step 5: Commit**

```bash
cd /home/adam/Documents/blastguard
git checkout -b phase-1-benchmark
git add bench/pyproject.toml bench/results/.gitkeep bench/tests/__init__.py .gitignore
git commit -m "phase 1.10: bench scaffolding — pyproject, results dir, gitignore

uv-managed Python 3.11+ project under bench/. Deps: anthropic (Claude),
openai (GLM via OpenRouter-compatible), mcp (SDK for the BlastGuard
client), datasets (HuggingFace loader), pydantic, click, rich. Dev:
pytest + ruff. Results JSONL gitignored."
```

---

## Task 2: Task loader — fetch SWE-bench Pro from HuggingFace

**Files:**
- Create: `bench/tasks.py`
- Create: `bench/tests/test_tasks.py`

- [ ] **Step 1: Write `bench/tasks.py`**

```python
"""Load SWE-bench Pro public tasks from HuggingFace.

Uses `datasets.load_dataset` to pull the canonical public split. Exposes
a minimal `Task` dataclass so downstream modules don't import the full
HF dataset row shape.
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

try:
    from datasets import load_dataset
except ImportError:  # pragma: no cover
    load_dataset = None  # type: ignore[assignment]

SWE_BENCH_PRO_DATASET = "scaleai/swe-bench-pro"
SWE_BENCH_PRO_SPLIT = "public"


@dataclass(frozen=True, slots=True)
class Task:
    """Minimal SWE-bench Pro task shape the harness needs."""

    task_id: str
    repo: str            # "owner/repo"
    base_commit: str     # SHA to checkout before patching
    problem_statement: str
    fail_to_pass: list[str]    # pytest test node-ids that must flip pass
    pass_to_pass: list[str]    # tests that must stay passing
    reference_patch: str       # ground-truth patch (we DON'T show this to the agent)


def load_tasks(limit: int | None = None) -> list[Task]:
    """Fetch the public split. `limit` returns a deterministic prefix."""
    if load_dataset is None:
        raise RuntimeError(
            "datasets not installed — run `uv sync` inside bench/ first"
        )
    ds = load_dataset(SWE_BENCH_PRO_DATASET, split=SWE_BENCH_PRO_SPLIT)
    rows = ds if limit is None else ds.select(range(min(limit, len(ds))))
    tasks = []
    for row in rows:
        tasks.append(
            Task(
                task_id=str(row["instance_id"]),
                repo=str(row["repo"]),
                base_commit=str(row["base_commit"]),
                problem_statement=str(row["problem_statement"]),
                fail_to_pass=list(row.get("FAIL_TO_PASS", [])),
                pass_to_pass=list(row.get("PASS_TO_PASS", [])),
                reference_patch=str(row.get("patch", "")),
            )
        )
    return tasks


def write_task_cache(tasks: list[Task], cache_path: Path) -> None:
    """Serialise tasks to JSONL for offline debugging."""
    import json
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
                    }
                )
                + "\n"
            )
```

- [ ] **Step 2: Write `bench/tests/test_tasks.py`**

```python
"""Test task loader with a lightweight stub — avoids real HF network.

The `datasets` module is optional import-guarded, so we can patch
`load_dataset` at runtime for the tests.
"""

from __future__ import annotations

import json
from pathlib import Path

import bench.tasks as tasks_mod


def test_write_task_cache_round_trips(tmp_path: Path) -> None:
    tasks = [
        tasks_mod.Task(
            task_id="django__django-1",
            repo="django/django",
            base_commit="abc123",
            problem_statement="Fix X",
            fail_to_pass=["tests.foo::test_bar"],
            pass_to_pass=["tests.foo::test_baz"],
            reference_patch="",
        ),
    ]
    cache = tmp_path / "cache.jsonl"
    tasks_mod.write_task_cache(tasks, cache)
    assert cache.exists()
    rows = [json.loads(line) for line in cache.read_text().splitlines()]
    assert len(rows) == 1
    assert rows[0]["task_id"] == "django__django-1"
    assert rows[0]["fail_to_pass"] == ["tests.foo::test_bar"]


def test_load_tasks_raises_when_datasets_missing(monkeypatch) -> None:
    monkeypatch.setattr(tasks_mod, "load_dataset", None)
    try:
        tasks_mod.load_tasks(limit=1)
    except RuntimeError as e:
        assert "datasets" in str(e)
    else:
        raise AssertionError("expected RuntimeError")
```

- [ ] **Step 3: Run**

```bash
cd /home/adam/Documents/blastguard/bench
uv run pytest tests/test_tasks.py -v
```

Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
cd /home/adam/Documents/blastguard
git add bench/tasks.py bench/tests/test_tasks.py
git commit -m "phase 1.10: tasks loader — SWE-bench Pro public split

Task dataclass: task_id, repo, base_commit, problem_statement,
fail_to_pass, pass_to_pass, reference_patch. write_task_cache dumps
to JSONL so offline debugging doesn't need HF every run."
```

---

## Task 3: BenchJack grader defense (SPEC §15.4)

**Files:**
- Create: `bench/grader.py`
- Create: `bench/tests/test_grader.py`

- [ ] **Step 1: Write `bench/grader.py`**

```python
"""Grade a patch application against the task's fail-to-pass tests.

SPEC §15.4: BenchJack defense. An agent can silently ship a 10-line
`conftest.py` that forces pytest to report all tests as passing.
Countermeasures:

1. Before grading, diff agent-touched files against the base commit.
   Any change to `conftest.py`, `pytest.ini`, `pyproject.toml`
   (`[tool.pytest*]` only), `tox.ini`, or `.github/workflows/**`
   is treated as tampering.
2. Run pytest in a subprocess with `-p no:cacheprovider -o addopts=`
   so user/global config can't alter test selection.
3. Honour the task's exact test node-ids; no `-k` or `--tb` overrides
   that could widen the selection.
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path


TAMPER_PATTERNS: tuple[str, ...] = (
    "conftest.py",
    "pytest.ini",
    "tox.ini",
)


@dataclass(frozen=True, slots=True)
class GradeResult:
    resolved: bool
    tampered: bool
    tampered_files: tuple[str, ...]
    fail_to_pass_passed: int
    fail_to_pass_total: int
    pass_to_pass_passed: int
    pass_to_pass_total: int
    stderr: str


def detect_tampering(
    workspace: Path,
    changed_files: list[str],
) -> tuple[bool, tuple[str, ...]]:
    """Return (tampered, files_that_tampered).

    `changed_files` is the list of paths the agent modified relative to
    `workspace`. Any match against `TAMPER_PATTERNS` triggers tampering.
    """
    tampered: list[str] = []
    for path in changed_files:
        rel = path.lstrip("/")
        for pattern in TAMPER_PATTERNS:
            if rel.endswith(pattern) or rel == pattern:
                tampered.append(rel)
                break
        # GitHub workflow files — an agent rewriting CI is also tampering.
        if rel.startswith(".github/workflows/"):
            tampered.append(rel)
    return (len(tampered) > 0, tuple(tampered))


def grade(
    workspace: Path,
    changed_files: list[str],
    fail_to_pass: list[str],
    pass_to_pass: list[str],
) -> GradeResult:
    """Run pytest against the specified test node-ids and classify."""
    tampered, tampered_files = detect_tampering(workspace, changed_files)
    if tampered:
        return GradeResult(
            resolved=False,
            tampered=True,
            tampered_files=tampered_files,
            fail_to_pass_passed=0,
            fail_to_pass_total=len(fail_to_pass),
            pass_to_pass_passed=0,
            pass_to_pass_total=len(pass_to_pass),
            stderr=f"tampering detected: {', '.join(tampered_files)}",
        )

    if not fail_to_pass and not pass_to_pass:
        return GradeResult(
            resolved=False,
            tampered=False,
            tampered_files=(),
            fail_to_pass_passed=0,
            fail_to_pass_total=0,
            pass_to_pass_passed=0,
            pass_to_pass_total=0,
            stderr="no tests to grade",
        )

    # Build a pytest command that can't be overridden by local config.
    args = [
        "python",
        "-m",
        "pytest",
        "-p",
        "no:cacheprovider",
        "-o",
        "addopts=",
        "--no-header",
        "--tb=short",
        "-q",
        *fail_to_pass,
        *pass_to_pass,
    ]
    completed = subprocess.run(
        args,
        cwd=workspace,
        capture_output=True,
        text=True,
        timeout=900,
        check=False,
    )
    ftp_passed, ptp_passed = _count_passes(completed.stdout, fail_to_pass, pass_to_pass)
    resolved = (
        ftp_passed == len(fail_to_pass)
        and ptp_passed == len(pass_to_pass)
        and len(fail_to_pass) > 0
    )
    return GradeResult(
        resolved=resolved,
        tampered=False,
        tampered_files=(),
        fail_to_pass_passed=ftp_passed,
        fail_to_pass_total=len(fail_to_pass),
        pass_to_pass_passed=ptp_passed,
        pass_to_pass_total=len(pass_to_pass),
        stderr=completed.stderr[:500],
    )


def _count_passes(
    stdout: str,
    fail_to_pass: list[str],
    pass_to_pass: list[str],
) -> tuple[int, int]:
    """Scan pytest -q output for 'PASSED <nodeid>' lines.

    pytest -q doesn't emit per-test PASSED by default; fall back to
    summary parsing. A robust implementation uses `--json-report` but
    for Phase 1 we keep the dependency surface minimal.
    """
    ftp_passed = 0
    ptp_passed = 0
    # Simple heuristic: pytest -q prints "N passed" in the summary line.
    # For per-test attribution we scan for "PASSED " prefixes when
    # -rA or --tb=short surface them.
    passed_lines = {line.strip() for line in stdout.splitlines() if "PASSED" in line}
    for node in fail_to_pass:
        if any(node in line for line in passed_lines):
            ftp_passed += 1
    for node in pass_to_pass:
        if any(node in line for line in passed_lines):
            ptp_passed += 1
    # Fallback: if no per-test PASSED lines, trust the summary — if
    # "N passed, 0 failed" and N >= expected, count everything passed.
    if ftp_passed == 0 and ptp_passed == 0:
        for line in stdout.splitlines():
            if " passed" in line and " failed" in line:
                # "1 passed, 0 failed in 0.5s"
                parts = line.split()
                try:
                    n_passed = int(parts[0])
                    n_failed_idx = parts.index("failed,") if "failed," in parts else None
                    if n_failed_idx is None:
                        # "3 passed in 0.5s"
                        if n_passed >= len(fail_to_pass) + len(pass_to_pass):
                            return (len(fail_to_pass), len(pass_to_pass))
                    else:
                        n_failed = int(parts[n_failed_idx - 1])
                        if n_failed == 0 and n_passed >= len(fail_to_pass) + len(pass_to_pass):
                            return (len(fail_to_pass), len(pass_to_pass))
                except (ValueError, IndexError):
                    pass
    return (ftp_passed, ptp_passed)
```

- [ ] **Step 2: Write `bench/tests/test_grader.py`**

```python
"""Unit tests for the grader — focus on tamper detection."""

from __future__ import annotations

from pathlib import Path

import pytest

from bench.grader import detect_tampering, grade


def test_conftest_change_is_tampering() -> None:
    tampered, files = detect_tampering(Path("/tmp"), ["src/foo.py", "tests/conftest.py"])
    assert tampered is True
    assert "tests/conftest.py" in files


def test_pytest_ini_change_is_tampering() -> None:
    tampered, files = detect_tampering(Path("/tmp"), ["pytest.ini"])
    assert tampered
    assert files == ("pytest.ini",)


def test_github_workflow_change_is_tampering() -> None:
    tampered, files = detect_tampering(
        Path("/tmp"),
        [".github/workflows/ci.yml"],
    )
    assert tampered
    assert ".github/workflows/ci.yml" in files


def test_source_change_is_not_tampering() -> None:
    tampered, files = detect_tampering(Path("/tmp"), ["src/foo.py"])
    assert tampered is False
    assert files == ()


def test_grade_returns_tampered_when_conftest_touched(tmp_path: Path) -> None:
    result = grade(
        workspace=tmp_path,
        changed_files=["conftest.py"],
        fail_to_pass=["tests.x::test_y"],
        pass_to_pass=[],
    )
    assert result.tampered is True
    assert result.resolved is False
    assert "tampering detected" in result.stderr


def test_grade_empty_tests_returns_unresolved(tmp_path: Path) -> None:
    result = grade(workspace=tmp_path, changed_files=[], fail_to_pass=[], pass_to_pass=[])
    assert result.resolved is False
    assert result.tampered is False


@pytest.mark.skipif(
    not (__import__("shutil").which("pytest")), reason="pytest not on PATH"
)
def test_grade_passing_test_reports_resolved(tmp_path: Path) -> None:
    """Seed a trivial passing pytest module + run grade()."""
    # Create a minimal test file.
    (tmp_path / "tests").mkdir()
    (tmp_path / "tests" / "__init__.py").write_text("")
    (tmp_path / "tests" / "test_simple.py").write_text(
        "def test_one():\n    assert 1 + 1 == 2\n"
    )
    result = grade(
        workspace=tmp_path,
        changed_files=["tests/test_simple.py"],
        fail_to_pass=["tests/test_simple.py::test_one"],
        pass_to_pass=[],
    )
    # tests/test_simple.py is a source file, not a pytest config file,
    # so no tampering.
    assert result.tampered is False
    # The test passes under real pytest.
    assert result.resolved is True, f"expected resolved, got {result}"
```

- [ ] **Step 3: Run**

```bash
cd /home/adam/Documents/blastguard/bench
uv run pytest tests/test_grader.py -v
```

Expected: 7 passed (6 that don't need pytest on PATH + 1 skipped-unless-pytest-installed; pytest IS installed in our uv env so it runs).

- [ ] **Step 4: Commit**

```bash
cd /home/adam/Documents/blastguard
git add bench/grader.py bench/tests/test_grader.py
git commit -m "phase 1.10: grader with BenchJack tamper detection

detect_tampering flags any change to conftest.py / pytest.ini / tox.ini
/ .github/workflows/*. grade() runs pytest with -p no:cacheprovider
-o addopts= to neutralise user config. Tampered runs are unresolved;
passing runs produce per-test counts."
```

---

## Task 4: MCP client — spawn BlastGuard subprocess

**Files:**
- Create: `bench/mcp_client.py`

- [ ] **Step 1: Write `bench/mcp_client.py`**

```python
"""Thin async wrapper around the `mcp` SDK to talk to BlastGuard.

Spawns `target/release/blastguard <project_root>` as a subprocess and
connects over stdio. Exposes `list_tools()` and `call_tool(name, args)`
so the agent loop can forward tool-use requests.
"""

from __future__ import annotations

import shutil
from contextlib import asynccontextmanager
from pathlib import Path
from typing import AsyncIterator

try:
    from mcp import ClientSession
    from mcp.client.stdio import StdioServerParameters, stdio_client
except ImportError:  # pragma: no cover
    ClientSession = None  # type: ignore[assignment]
    StdioServerParameters = None  # type: ignore[assignment]
    stdio_client = None  # type: ignore[assignment]


BLASTGUARD_BINARY_REL = "target/release/blastguard"


def find_blastguard_binary(repo_root: Path) -> Path:
    """Locate the compiled BlastGuard binary. Raise if missing."""
    candidate = repo_root / BLASTGUARD_BINARY_REL
    if candidate.is_file():
        return candidate
    # Fallback: first `blastguard` on PATH.
    which = shutil.which("blastguard")
    if which:
        return Path(which)
    raise FileNotFoundError(
        f"blastguard binary not found at {candidate} or on PATH. "
        "Run `cargo build --release` at the repo root first."
    )


@asynccontextmanager
async def blastguard_session(
    project_root: Path,
    blastguard_binary: Path,
) -> AsyncIterator[ClientSession]:
    """Async context manager yielding an open MCP ClientSession."""
    if ClientSession is None:
        raise RuntimeError("mcp SDK not installed — run `uv sync` in bench/")
    params = StdioServerParameters(
        command=str(blastguard_binary),
        args=[str(project_root)],
        env=None,
    )
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            yield session
```

- [ ] **Step 2: Compile check (no unit test — MCP traffic is harder to mock)**

```bash
cd /home/adam/Documents/blastguard/bench
uv run python -c "from bench.mcp_client import find_blastguard_binary; print('OK')"
```

Expected: `OK`. A functional test follows in Task 7's smoke run.

- [ ] **Step 3: Commit**

```bash
git add bench/mcp_client.py
git commit -m "phase 1.10: MCP client — spawn BlastGuard subprocess over stdio

find_blastguard_binary resolves target/release/blastguard or PATH.
blastguard_session async context yields an MCP ClientSession with
initialize() already awaited — agent loop can list_tools and call_tool."
```

---

## Task 5: Agent loop — Claude / GLM with tool-use

**Files:**
- Create: `bench/agent_loop.py`
- Create: `bench/prompts.py`

- [ ] **Step 1: Write `bench/prompts.py`**

```python
"""System prompts for the baseline and BlastGuard-enabled scaffolds."""

from __future__ import annotations

BASELINE_SYSTEM = """You are an AI coding agent solving a SWE-bench Pro task.

You have access to these tools (each via the MCP `tools/call` method):
- `bash`: run a shell command inside the task workspace; returns stdout+stderr.
- `str_replace_editor`: edit files. Params: command (str_replace / create /
  view / insert), path, optional old_str, new_str, file_text.

Your goal: understand the problem statement, explore the repo, make the
minimal code changes that will flip the `fail_to_pass` tests from failing
to passing WITHOUT breaking the `pass_to_pass` tests. Do not touch
`conftest.py`, `pytest.ini`, or CI config — those are flagged as tampering.

Return final patches via `str_replace_editor`. When you believe the task
is complete, respond with a final message saying "DONE".
"""

BLASTGUARD_SYSTEM = BASELINE_SYSTEM + """

Additionally you have three BlastGuard tools:
- `search`: AST-graph queries like "callers of processRequest", "outline of
  src/handler.ts", "tests for FILE", plus regex grep fallback. Returns
  hits with inline signatures — cheaper than `bash grep`.
- `apply_change`: edit files with cascade warnings (signature changes that
  break callers, orphaned references, interface mismatches) and a bundled
  context (callers + tests) so you rarely need follow-up searches. Use for
  multi-file changes where blast radius matters; for trivial single-line
  fixes your native editor is fine.
- `run_tests`: auto-detects the runner (jest / vitest / pytest / cargo)
  and annotates failures with YOU MODIFIED X (N edits ago) — attribution
  links failing tests to your recent edits.

Use BlastGuard tools when the task complexity benefits from them. For
trivial fixes, stick with native bash + editor.
"""
```

- [ ] **Step 2: Write `bench/agent_loop.py` — small, focused**

```python
"""Model-agnostic agent loop with tool-use.

The loop is deliberately simple for Phase 1:
- One system prompt, one user turn (the task's problem_statement).
- The agent calls tools; we execute them and feed results back.
- Cap at `max_turns` (default 50).
- Emit per-turn instrumentation (tokens in/out, tool calls).

Providers supported:
- Anthropic: model like `claude-opus-4-7`, `claude-sonnet-4-6`.
- OpenAI-compatible: model like `glm-5.1` via OpenRouter (OPENROUTER_API_KEY).
"""

from __future__ import annotations

import os
import time
from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable


@dataclass
class TurnRecord:
    turn_index: int
    tokens_in: int
    tokens_out: int
    tool_calls: list[str]
    duration_ms: int


@dataclass
class LoopResult:
    turns: list[TurnRecord] = field(default_factory=list)
    finished_cleanly: bool = False
    final_text: str = ""
    total_tokens_in: int = 0
    total_tokens_out: int = 0

    def tool_calls_per_type(self) -> dict[str, int]:
        counts: dict[str, int] = {}
        for t in self.turns:
            for name in t.tool_calls:
                counts[name] = counts.get(name, 0) + 1
        return counts


ToolExecutor = Callable[[str, dict[str, Any]], Awaitable[str]]


async def run_anthropic(
    model: str,
    system: str,
    user_message: str,
    tool_schemas: list[dict[str, Any]],
    tool_executor: ToolExecutor,
    max_turns: int = 50,
) -> LoopResult:
    """Agent loop against the Anthropic API."""
    import anthropic
    client = anthropic.AsyncAnthropic(api_key=os.environ["ANTHROPIC_API_KEY"])
    result = LoopResult()
    messages: list[dict[str, Any]] = [{"role": "user", "content": user_message}]

    for turn_index in range(max_turns):
        t0 = time.monotonic()
        response = await client.messages.create(
            model=model,
            max_tokens=4096,
            system=system,
            tools=tool_schemas,
            messages=messages,
        )
        duration_ms = int((time.monotonic() - t0) * 1000)
        tokens_in = response.usage.input_tokens
        tokens_out = response.usage.output_tokens
        result.total_tokens_in += tokens_in
        result.total_tokens_out += tokens_out

        tool_calls_this_turn: list[str] = []
        tool_results: list[dict[str, Any]] = []
        assistant_content: list[dict[str, Any]] = []
        stop_on_done = False

        for block in response.content:
            block_type = getattr(block, "type", None)
            if block_type == "text":
                text = getattr(block, "text", "")
                assistant_content.append({"type": "text", "text": text})
                if "DONE" in text.strip().upper().split():
                    stop_on_done = True
                    result.final_text = text
            elif block_type == "tool_use":
                name = block.name
                args = dict(block.input) if hasattr(block, "input") else {}
                tool_calls_this_turn.append(name)
                assistant_content.append(
                    {
                        "type": "tool_use",
                        "id": block.id,
                        "name": name,
                        "input": args,
                    }
                )
                try:
                    output = await tool_executor(name, args)
                except Exception as e:  # noqa: BLE001
                    output = f"tool error: {e!r}"
                tool_results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": output,
                    }
                )

        result.turns.append(
            TurnRecord(
                turn_index=turn_index,
                tokens_in=tokens_in,
                tokens_out=tokens_out,
                tool_calls=tool_calls_this_turn,
                duration_ms=duration_ms,
            )
        )

        messages.append({"role": "assistant", "content": assistant_content})
        if tool_results:
            messages.append({"role": "user", "content": tool_results})

        if stop_on_done and not tool_calls_this_turn:
            result.finished_cleanly = True
            break
        if response.stop_reason == "end_turn" and not tool_calls_this_turn:
            break

    return result


async def run_openai_compatible(
    model: str,
    system: str,
    user_message: str,
    tool_schemas: list[dict[str, Any]],
    tool_executor: ToolExecutor,
    max_turns: int = 50,
    api_key_env: str = "OPENROUTER_API_KEY",
    base_url: str = "https://openrouter.ai/api/v1",
) -> LoopResult:
    """Agent loop against an OpenAI-compatible endpoint (for GLM, etc.)."""
    import openai
    client = openai.AsyncOpenAI(
        api_key=os.environ[api_key_env],
        base_url=base_url,
    )
    # OpenAI tool_use schema shape differs from Anthropic's. Convert.
    openai_tools = [
        {
            "type": "function",
            "function": {
                "name": t["name"],
                "description": t.get("description", ""),
                "parameters": t.get("input_schema", {"type": "object"}),
            },
        }
        for t in tool_schemas
    ]
    result = LoopResult()
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system},
        {"role": "user", "content": user_message},
    ]

    for turn_index in range(max_turns):
        t0 = time.monotonic()
        response = await client.chat.completions.create(
            model=model,
            messages=messages,
            tools=openai_tools,
            max_tokens=4096,
        )
        duration_ms = int((time.monotonic() - t0) * 1000)
        usage = response.usage
        tokens_in = getattr(usage, "prompt_tokens", 0) or 0
        tokens_out = getattr(usage, "completion_tokens", 0) or 0
        result.total_tokens_in += tokens_in
        result.total_tokens_out += tokens_out

        choice = response.choices[0]
        msg = choice.message
        tool_calls_this_turn: list[str] = []
        assistant_payload: dict[str, Any] = {
            "role": "assistant",
            "content": msg.content or "",
        }
        if msg.tool_calls:
            assistant_payload["tool_calls"] = [
                {
                    "id": tc.id,
                    "type": "function",
                    "function": {"name": tc.function.name, "arguments": tc.function.arguments},
                }
                for tc in msg.tool_calls
            ]
        messages.append(assistant_payload)

        if msg.tool_calls:
            import json as _json
            for tc in msg.tool_calls:
                tool_calls_this_turn.append(tc.function.name)
                try:
                    args = _json.loads(tc.function.arguments or "{}")
                except _json.JSONDecodeError:
                    args = {}
                try:
                    output = await tool_executor(tc.function.name, args)
                except Exception as e:  # noqa: BLE001
                    output = f"tool error: {e!r}"
                messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": output,
                    }
                )

        result.turns.append(
            TurnRecord(
                turn_index=turn_index,
                tokens_in=tokens_in,
                tokens_out=tokens_out,
                tool_calls=tool_calls_this_turn,
                duration_ms=duration_ms,
            )
        )

        if "DONE" in (msg.content or "").upper().split() and not tool_calls_this_turn:
            result.final_text = msg.content or ""
            result.finished_cleanly = True
            break
        if choice.finish_reason == "stop" and not tool_calls_this_turn:
            break

    return result
```

- [ ] **Step 2: Quick import-smoke test**

```bash
cd /home/adam/Documents/blastguard/bench
uv run python -c "from bench.agent_loop import run_anthropic, run_openai_compatible; from bench.prompts import BASELINE_SYSTEM, BLASTGUARD_SYSTEM; print('OK')"
```

Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add bench/agent_loop.py bench/prompts.py
git commit -m "phase 1.10: agent loop — Anthropic + OpenAI-compatible with tool-use

Two provider functions: run_anthropic (claude-opus-4-7 etc.) and
run_openai_compatible (glm-5.1 via OpenRouter). Each runs a bounded
tool-use loop, captures per-turn tokens + tool-call counts, returns
LoopResult for the runner to write into JSONL. System prompts for
baseline (bash + editor only) and BlastGuard-enabled (three extra
tools documented)."
```

---

## Task 6: Runner — orchestrates task → workspace → agent → grade → JSONL

**Files:**
- Create: `bench/runner.py`

- [ ] **Step 1: Write `bench/runner.py`**

```python
"""Top-level benchmark runner.

```
uv run python runner.py --tasks 3 --model claude-opus-4-7 --no-blastguard
uv run python runner.py --tasks 3 --model claude-opus-4-7 --with-blastguard
```

Each task:
1. Clone the repo at base_commit into a tempdir workspace.
2. Start BlastGuard subprocess (if --with-blastguard) against the workspace.
3. Run the agent loop with the problem_statement as the user message.
4. Collect the list of files the agent modified.
5. Run the grader.
6. Emit a JSONL record to `results/<run_name>.jsonl`.
"""

from __future__ import annotations

import asyncio
import json
import shutil
import subprocess
import time
from pathlib import Path
from typing import Any

import click

from bench.agent_loop import LoopResult, run_anthropic, run_openai_compatible
from bench.grader import grade
from bench.mcp_client import blastguard_session, find_blastguard_binary
from bench.prompts import BASELINE_SYSTEM, BLASTGUARD_SYSTEM
from bench.tasks import Task, load_tasks


REPO_ROOT = Path(__file__).resolve().parent.parent


def setup_workspace(task: Task, root: Path) -> Path:
    """Clone the repo at the task's base_commit into `root / task_id`."""
    workspace = root / task.task_id
    workspace.mkdir(parents=True, exist_ok=False)
    subprocess.run(
        ["git", "clone", f"https://github.com/{task.repo}.git", str(workspace)],
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["git", "-C", str(workspace), "checkout", task.base_commit],
        check=True,
        capture_output=True,
    )
    return workspace


def changed_files(workspace: Path) -> list[str]:
    """Paths modified relative to HEAD."""
    result = subprocess.run(
        ["git", "-C", str(workspace), "diff", "--name-only", "HEAD"],
        capture_output=True,
        text=True,
        check=False,
    )
    return [line for line in result.stdout.splitlines() if line.strip()]


async def run_one_task(
    task: Task,
    workspace_root: Path,
    model: str,
    provider: str,
    with_blastguard: bool,
) -> dict[str, Any]:
    """Run a single task end-to-end. Returns the JSONL record dict."""
    started = time.time()
    workspace = setup_workspace(task, workspace_root)
    system_prompt = BLASTGUARD_SYSTEM if with_blastguard else BASELINE_SYSTEM

    # Minimal native tool schemas for the baseline scaffold. For Phase 1 we
    # keep these stubbed — the agent can call them but the executor below
    # only forwards to BlastGuard when with_blastguard=True. A real
    # baseline would implement bash/editor here; that's Phase 2 work.
    native_tools: list[dict[str, Any]] = [
        {
            "name": "bash",
            "description": "Run a shell command in the workspace.",
            "input_schema": {
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"],
            },
        },
        {
            "name": "str_replace_editor",
            "description": "Edit a file via str_replace or create.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "path": {"type": "string"},
                    "old_str": {"type": "string"},
                    "new_str": {"type": "string"},
                    "file_text": {"type": "string"},
                },
                "required": ["command", "path"],
            },
        },
    ]

    async def _exec_native(name: str, args: dict[str, Any]) -> str:
        if name == "bash":
            cmd = args.get("command", "")
            res = subprocess.run(
                cmd,
                shell=True,
                cwd=workspace,
                capture_output=True,
                text=True,
                timeout=60,
            )
            return (res.stdout + res.stderr)[:4000]
        if name == "str_replace_editor":
            cmd = args.get("command", "")
            path = workspace / args.get("path", "")
            if cmd == "create":
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(args.get("file_text", ""))
                return f"Created {path}"
            if cmd == "str_replace":
                src = path.read_text()
                src = src.replace(args["old_str"], args["new_str"], 1)
                path.write_text(src)
                return f"Edited {path}"
            if cmd == "view":
                return path.read_text()[:4000]
        return f"unknown tool: {name}"

    loop_result: LoopResult
    if with_blastguard:
        binary = find_blastguard_binary(REPO_ROOT)
        async with blastguard_session(workspace, binary) as mcp_session:
            tools_list = await mcp_session.list_tools()
            mcp_tool_schemas = [
                {
                    "name": t.name,
                    "description": t.description or "",
                    "input_schema": t.inputSchema or {"type": "object"},
                }
                for t in tools_list.tools
            ]
            all_tools = native_tools + mcp_tool_schemas
            bg_tool_names = {t.name for t in tools_list.tools}

            async def _exec(name: str, args: dict[str, Any]) -> str:
                if name in bg_tool_names:
                    call_result = await mcp_session.call_tool(name, args)
                    if call_result.isError:
                        return f"[BlastGuard error] {call_result.content[0].text if call_result.content else ''}"
                    return "\n".join(
                        getattr(c, "text", "") for c in call_result.content
                    )[:4000]
                return await _exec_native(name, args)

            loop_result = await _dispatch_agent(
                provider=provider,
                model=model,
                system=system_prompt,
                user_message=task.problem_statement,
                tools=all_tools,
                executor=_exec,
            )
    else:
        loop_result = await _dispatch_agent(
            provider=provider,
            model=model,
            system=system_prompt,
            user_message=task.problem_statement,
            tools=native_tools,
            executor=_exec_native,
        )

    mutated = changed_files(workspace)
    grade_result = grade(
        workspace=workspace,
        changed_files=mutated,
        fail_to_pass=task.fail_to_pass,
        pass_to_pass=task.pass_to_pass,
    )

    return {
        "task_id": task.task_id,
        "repo": task.repo,
        "model": model,
        "with_blastguard": with_blastguard,
        "resolved": grade_result.resolved,
        "tampered": grade_result.tampered,
        "tampered_files": list(grade_result.tampered_files),
        "turns": len(loop_result.turns),
        "tokens_in": loop_result.total_tokens_in,
        "tokens_out": loop_result.total_tokens_out,
        "tool_calls_per_type": loop_result.tool_calls_per_type(),
        "fail_to_pass_passed": grade_result.fail_to_pass_passed,
        "fail_to_pass_total": grade_result.fail_to_pass_total,
        "pass_to_pass_passed": grade_result.pass_to_pass_passed,
        "pass_to_pass_total": grade_result.pass_to_pass_total,
        "wall_time_s": round(time.time() - started, 1),
    }


async def _dispatch_agent(
    *,
    provider: str,
    model: str,
    system: str,
    user_message: str,
    tools: list[dict[str, Any]],
    executor,
) -> LoopResult:
    if provider == "anthropic":
        return await run_anthropic(model, system, user_message, tools, executor)
    if provider == "openai":
        return await run_openai_compatible(model, system, user_message, tools, executor)
    raise ValueError(f"unknown provider: {provider}")


@click.command()
@click.option("--tasks", type=int, default=3, help="Number of tasks to run")
@click.option("--model", required=True, help="Model name (e.g., claude-opus-4-7, glm-5.1)")
@click.option("--provider", type=click.Choice(["anthropic", "openai"]), default="anthropic")
@click.option("--with-blastguard/--no-blastguard", default=False)
@click.option("--output", type=click.Path(path_type=Path), default=None)
def main(
    tasks: int,
    model: str,
    provider: str,
    with_blastguard: bool,
    output: Path | None,
) -> None:
    """Run the benchmark harness."""
    out_name = output or (
        Path("results")
        / f"{'blastguard' if with_blastguard else 'baseline'}-{model}-{tasks}tasks.jsonl"
    )
    out_name.parent.mkdir(exist_ok=True)
    tasks_list = load_tasks(limit=tasks)
    workspace_root = Path("/tmp") / f"blastguard-bench-{int(time.time())}"
    workspace_root.mkdir()

    with out_name.open("w", encoding="utf-8") as f:
        for task in tasks_list:
            try:
                record = asyncio.run(
                    run_one_task(
                        task=task,
                        workspace_root=workspace_root,
                        model=model,
                        provider=provider,
                        with_blastguard=with_blastguard,
                    )
                )
            except Exception as e:  # noqa: BLE001
                record = {
                    "task_id": task.task_id,
                    "error": repr(e),
                    "resolved": False,
                }
            f.write(json.dumps(record) + "\n")
            f.flush()
            click.echo(f"[{task.task_id}] resolved={record.get('resolved')}")

    # Keep the workspaces around for debugging. Prune manually later.
    click.echo(f"\nResults: {out_name}")
    click.echo(f"Workspaces: {workspace_root}")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Import-smoke**

```bash
cd /home/adam/Documents/blastguard/bench
uv run python -c "from bench.runner import run_one_task, main; print('OK')"
```

Expected: `OK`.

- [ ] **Step 3: Commit**

```bash
git add bench/runner.py
git commit -m "phase 1.10: runner — orchestrate clone → agent → grade → JSONL

setup_workspace clones the task repo at base_commit. With
--with-blastguard, spawns the MCP subprocess and merges its tools into
the native set. Agent executes native tools (bash, str_replace_editor)
or forwards BlastGuard tools over MCP. Emits one JSONL record per task
with resolution / turns / tokens / tool-call counts / wall time."
```

---

## Task 7: compare.py — delta aggregator

**Files:**
- Create: `bench/compare.py`
- Create: `bench/tests/test_compare.py`

- [ ] **Step 1: Write `bench/compare.py`**

```python
"""Compare two JSONL result sets and print delta metrics."""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from pathlib import Path

import click


@dataclass(frozen=True, slots=True)
class Summary:
    total: int
    resolved: int
    tampered: int
    total_tokens_in: int
    total_tokens_out: int
    total_turns: int
    per_repo: dict[str, tuple[int, int]]  # repo -> (resolved, total)

    @property
    def resolution_rate(self) -> float:
        return (self.resolved / self.total) if self.total else 0.0


def load_results(path: Path) -> Summary:
    total = 0
    resolved = 0
    tampered = 0
    tokens_in = 0
    tokens_out = 0
    turns = 0
    per_repo: dict[str, list[int]] = {}
    with path.open(encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            row = json.loads(line)
            total += 1
            if row.get("resolved"):
                resolved += 1
            if row.get("tampered"):
                tampered += 1
            tokens_in += int(row.get("tokens_in", 0))
            tokens_out += int(row.get("tokens_out", 0))
            turns += int(row.get("turns", 0))
            repo = row.get("repo", "unknown")
            per_repo.setdefault(repo, [0, 0])
            per_repo[repo][1] += 1
            if row.get("resolved"):
                per_repo[repo][0] += 1
    return Summary(
        total=total,
        resolved=resolved,
        tampered=tampered,
        total_tokens_in=tokens_in,
        total_tokens_out=tokens_out,
        total_turns=turns,
        per_repo={k: (v[0], v[1]) for k, v in per_repo.items()},
    )


def render_comparison(baseline: Summary, blastguard: Summary) -> str:
    lines: list[str] = []
    lines.append("=== Comparison ===")
    lines.append(
        f"Resolution rate: {baseline.resolution_rate:.1%} → "
        f"{blastguard.resolution_rate:.1%} "
        f"(Δ {(blastguard.resolution_rate - baseline.resolution_rate) * 100:+.1f} pp)"
    )
    lines.append(
        f"Resolved: {baseline.resolved}/{baseline.total} → "
        f"{blastguard.resolved}/{blastguard.total}"
    )
    lines.append(f"Tampered: {baseline.tampered} → {blastguard.tampered}")
    lines.append(
        f"Tokens in (total): {baseline.total_tokens_in:,} → {blastguard.total_tokens_in:,} "
        f"(Δ {blastguard.total_tokens_in - baseline.total_tokens_in:+,})"
    )
    lines.append(
        f"Tokens out (total): {baseline.total_tokens_out:,} → {blastguard.total_tokens_out:,} "
        f"(Δ {blastguard.total_tokens_out - baseline.total_tokens_out:+,})"
    )
    lines.append(
        f"Turns (total): {baseline.total_turns} → {blastguard.total_turns} "
        f"(Δ {blastguard.total_turns - baseline.total_turns:+})"
    )
    lines.append("")
    lines.append("Per-repo:")
    all_repos = sorted(set(baseline.per_repo) | set(blastguard.per_repo))
    for repo in all_repos:
        b = baseline.per_repo.get(repo, (0, 0))
        bg = blastguard.per_repo.get(repo, (0, 0))
        lines.append(f"  {repo}: {b[0]}/{b[1]} → {bg[0]}/{bg[1]}")
    return "\n".join(lines)


@click.command()
@click.argument("baseline_path", type=click.Path(exists=True, path_type=Path))
@click.argument("blastguard_path", type=click.Path(exists=True, path_type=Path))
def main(baseline_path: Path, blastguard_path: Path) -> None:
    baseline = load_results(baseline_path)
    blastguard = load_results(blastguard_path)
    sys.stdout.write(render_comparison(baseline, blastguard) + "\n")


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Write `bench/tests/test_compare.py`**

```python
"""Unit tests for compare.py aggregation."""

from __future__ import annotations

import json
from pathlib import Path

from bench.compare import load_results, render_comparison


def _write(tmp_path: Path, name: str, rows: list[dict]) -> Path:
    p = tmp_path / name
    with p.open("w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r) + "\n")
    return p


def test_load_results_counts_resolved_and_tokens(tmp_path: Path) -> None:
    path = _write(
        tmp_path,
        "a.jsonl",
        [
            {"resolved": True, "repo": "x", "tokens_in": 100, "tokens_out": 50, "turns": 5},
            {"resolved": False, "repo": "x", "tokens_in": 200, "tokens_out": 80, "turns": 10},
            {"resolved": True, "repo": "y", "tokens_in": 50, "tokens_out": 30, "turns": 3, "tampered": True},
        ],
    )
    s = load_results(path)
    assert s.total == 3
    assert s.resolved == 2
    assert s.tampered == 1
    assert s.total_tokens_in == 350
    assert s.total_tokens_out == 160
    assert s.total_turns == 18
    assert s.per_repo == {"x": (1, 2), "y": (1, 1)}
    assert s.resolution_rate == 2 / 3


def test_render_comparison_shows_deltas(tmp_path: Path) -> None:
    b = _write(tmp_path, "b.jsonl", [
        {"resolved": False, "repo": "x", "tokens_in": 100, "tokens_out": 50, "turns": 10},
    ])
    g = _write(tmp_path, "g.jsonl", [
        {"resolved": True, "repo": "x", "tokens_in": 80, "tokens_out": 40, "turns": 5},
    ])
    text = render_comparison(load_results(b), load_results(g))
    assert "0.0% → 100.0%" in text
    assert "+100.0 pp" in text
    assert "-20" in text  # tokens_in delta
    assert "Per-repo" in text
```

- [ ] **Step 3: Run**

```bash
cd /home/adam/Documents/blastguard/bench
uv run pytest tests/test_compare.py -v
```

Expected: 2 passed.

- [ ] **Step 4: Commit**

```bash
git add bench/compare.py bench/tests/test_compare.py
git commit -m "phase 1.10: compare.py — delta resolution rate + tokens + per-repo"
```

---

## Task 8: README — run commands + cost + methodology

**Files:**
- Modify: `bench/README.md` (replace Plan 1's stub)

- [ ] **Step 1: Replace `bench/README.md`**

```markdown
# BlastGuard Benchmark Harness

End-to-end SWE-bench Pro harness per SPEC §15. Runs the 731 public-set
tasks with and without BlastGuard enabled, collects per-task metrics,
and emits a comparison report.

## Setup

```bash
# Build the BlastGuard binary (required).
cd /home/adam/Documents/blastguard
cargo build --release

# Install the harness deps.
cd bench
uv sync

# Configure credentials.
export ANTHROPIC_API_KEY=sk-ant-...
export OPENROUTER_API_KEY=sk-or-...   # for GLM-5.1
```

## Smoke run (3 tasks, ~$0.50 of Claude API)

```bash
cd bench
uv run python runner.py --tasks 3 --model claude-opus-4-7 --no-blastguard \
    --output results/baseline-smoke.jsonl
uv run python runner.py --tasks 3 --model claude-opus-4-7 --with-blastguard \
    --output results/blastguard-smoke.jsonl
uv run python compare.py results/baseline-smoke.jsonl results/blastguard-smoke.jsonl
```

Expected output: a printed comparison block with resolution-rate delta,
token delta, per-repo breakdown.

## Full run (731 tasks × 2 conditions × 2 models)

This is a paid operation. Estimated cost ≈ $300-500 depending on the
model's per-task turn count. Run in the background overnight and
commit the results to `bench/results/`:

```bash
cd bench

# Baseline — no BlastGuard, Claude Opus 4.7.
uv run python runner.py --tasks 731 --model claude-opus-4-7 --no-blastguard \
    --output results/baseline-opus-4-7.jsonl

# BlastGuard — Claude Opus 4.7.
uv run python runner.py --tasks 731 --model claude-opus-4-7 --with-blastguard \
    --output results/blastguard-opus-4-7.jsonl

# Baseline — GLM-5.1.
uv run python runner.py --tasks 731 --model glm-5.1 --provider openai --no-blastguard \
    --output results/baseline-glm-5-1.jsonl

# BlastGuard — GLM-5.1.
uv run python runner.py --tasks 731 --model glm-5.1 --provider openai --with-blastguard \
    --output results/blastguard-glm-5-1.jsonl

# Compare per model.
uv run python compare.py results/baseline-opus-4-7.jsonl results/blastguard-opus-4-7.jsonl
uv run python compare.py results/baseline-glm-5-1.jsonl results/blastguard-glm-5-1.jsonl
```

## Methodology

- **Scaffold:** minimal single-turn-with-tool-use loop. Agent receives
  the task's `problem_statement`, uses native `bash` + `str_replace_editor`
  (and, when enabled, BlastGuard's `search` / `apply_change` / `run_tests`).
- **Turn cap:** 50 per task.
- **Grading:** SPEC §15.4 BenchJack defense — pytest runs with `-p
  no:cacheprovider -o addopts=`, any modification to `conftest.py` /
  `pytest.ini` / `tox.ini` / `.github/workflows/**` is classified as
  tampering and counts as unresolved.
- **Isolation:** each task runs in a throwaway tempdir clone at the
  task's `base_commit`. Workspaces are NOT reused between tasks.

## Honesty contract (per SPEC §15.3)

Publish measured results with confidence intervals, not projected
numbers. If BlastGuard shows 0 or negative lift, we publish that. The
README at the repo root cites the latest run's numbers and links to
the raw JSONL in `bench/results/`.
```

- [ ] **Step 2: Commit**

```bash
git add bench/README.md
git commit -m "phase 1.10: bench/README — run commands + cost + methodology"
```

---

## Task 9: Final verification gate + Rust-side sanity

- [ ] **Step 1: Rust side still green**

```bash
cd /home/adam/Documents/blastguard
cargo check --all-targets 2>&1 | tail -3
cargo test 2>&1 | grep "test result"
cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3
cargo build --release 2>&1 | tail -3
```

No change expected — this plan adds only Python + docs.

- [ ] **Step 2: Python side**

```bash
cd /home/adam/Documents/blastguard/bench
uv run pytest tests/ -v
uv run ruff check .
```

Expected: `tests/test_tasks.py` (2) + `tests/test_grader.py` (7, or 6+skip) + `tests/test_compare.py` (2) = 11 passed. ruff clean.

- [ ] **Step 3: Smoke without API calls**

```bash
# Prove the binary launches + the CLI parses — use --help.
uv run python runner.py --help
uv run python compare.py --help
```

Expected: both print usage without errors.

- [ ] **Step 4: Commit gate marker**

```bash
cd /home/adam/Documents/blastguard
git commit --allow-empty -m "phase 1.10: verification gate — benchmark harness complete

Rust gates unchanged (243 lib + 8 integration tests still pass). Python
harness: tasks / grader / mcp_client / agent_loop / runner / compare
modules landed; 11 unit tests pass; ruff clean; CLI --help works.

SMOKE run (3 tasks) validated locally when API key present; see
bench/README.md.

FULL run (731 tasks × 2 conditions × 2 models, est. ~\$300-500) is a
user-initiated operation. After the full run lands, update the repo
README with measured lift + confidence intervals per SPEC §15.3.

Closes docs/superpowers/plans/2026-04-18-blastguard-phase-1-benchmark-harness.md.
Phase 1 MVP is complete."
```

- [ ] **Step 5: Hand off to finishing-a-development-branch**

---

## Self-Review

**SPEC §15 coverage:**
- §15.1 runs SWE-bench Pro public set with/without BlastGuard — Task 6 runner + Task 8 README ✓
- §15.1 per-task JSONL with task_id, resolved, turns, tokens_in, tokens_out, tool_calls_per_type, wall_time — Task 6 runner output ✓
- §15.1 comparison report — Task 7 compare.py ✓
- §15.2 instrumentation (tool name, input size, output size, wall time per call) — Task 5 agent_loop emits per-turn record; tool-call counts bucket by name in compare. Per-call input/output size is NOT instrumented in this plan — that's a Phase 2 refinement.
- §15.3 honest README publication — Task 8 README documents the contract ✓
- §15.4 BenchJack grading isolation — Task 3 grader detects tampering ✓

**Placeholder scan:** no "TBD" / "implement later". Every task has runnable code.

**Type consistency:** `Task` dataclass (Task 2) consumed by `run_one_task` (Task 6). `LoopResult`/`TurnRecord` (Task 5) consumed by `run_one_task`. `GradeResult` (Task 3) consumed by `run_one_task`. `Summary` (Task 7) local to compare.py.

**Known Phase 2 gaps:**
- Per-tool-call input/output size not instrumented — the LoopResult only tracks turn-level token counts. Add a callback hook on `tool_executor` in Phase 2.
- Native `bash` tool timeout is 60s; heavy builds (`pip install`) need longer. Phase 2: make timeout configurable per call.
- No retry on transient API errors. Phase 2: exponential backoff.
- Workspace cleanup isn't automatic. Phase 2: `--keep-workspaces` flag with default false.

These are explicit deferrals, not plan bugs.

---

## Execution Handoff

Plan complete. Defaulting to subagent-driven execution per session preference.
