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

SWE_BENCH_PRO_DATASET = "ScaleAI/SWE-bench_Pro"
SWE_BENCH_PRO_SPLIT = "test"


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
