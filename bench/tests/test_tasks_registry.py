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
