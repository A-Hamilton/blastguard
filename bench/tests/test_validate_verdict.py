"""Tests for bench/validate_verdict.py — three-axis bench verdict tool."""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from bench.validate_verdict import load_baseline, load_judge, load_run


def _write_jsonl(path: Path, records: list[dict]) -> None:
    path.write_text("\n".join(json.dumps(r) for r in records) + "\n")


def test_load_run_reads_jsonl_records(tmp_path: Path) -> None:
    p = tmp_path / "run.jsonl"
    _write_jsonl(p, [
        {"task_id": "a", "arm": "raw", "seed": 1, "input_tokens": 100, "wall_seconds": 1.0,
         "final_answer": "ok", "stopped_reason": "done_marker"},
        {"task_id": "a", "arm": "blastguard", "seed": 1, "input_tokens": 50, "wall_seconds": 0.5,
         "final_answer": "ok", "stopped_reason": "done_marker"},
    ])
    records = load_run(p)
    assert len(records) == 2
    assert records[0]["arm"] == "raw"
    assert records[1]["arm"] == "blastguard"


def test_load_judge_returns_empty_when_missing(tmp_path: Path) -> None:
    assert load_judge(tmp_path / "nope.jsonl") == []


def test_load_baseline_returns_none_when_missing(tmp_path: Path) -> None:
    assert load_baseline(tmp_path / "nope.json") is None


def test_load_baseline_reads_json(tmp_path: Path) -> None:
    p = tmp_path / "baseline.json"
    p.write_text(json.dumps({"model": "x", "tasks": {}}))
    assert load_baseline(p) == {"model": "x", "tasks": {}}
