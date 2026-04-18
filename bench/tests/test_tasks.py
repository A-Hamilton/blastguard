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
            language="python",
            dockerhub_tag="",
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
