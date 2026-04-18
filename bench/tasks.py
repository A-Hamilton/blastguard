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
        language = str(row.get("repo_language", "")).lower()
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
