"""Unit tests for the multi-seed micro-bench aggregator."""

from __future__ import annotations

import json
from pathlib import Path

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
    # Raw arm total cost across 3 seeds of 1 task: 0.018, 0.020, 0.022 -> mean 0.020.
    # BG arm across same seeds: 0.010, 0.010, 0.010 -> mean 0.010.
    # Paired differences [0.008, 0.010, 0.012] mean 0.010, sample_std=0.002.
    # t(0.975, df=2)=4.303, half=0.002*4.303/sqrt(3)=0.00497, ci_low=0.00503 > 0.
    # The paired-difference CI does not include 0 at 95%, confirming BG cheaper.
    f = tmp_path / "y.jsonl"
    records = []
    for seed, (raw_cost, bg_cost) in enumerate(
        [(0.018, 0.010), (0.020, 0.010), (0.022, 0.010)], start=1
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
