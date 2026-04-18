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
            {
                "resolved": True, "repo": "y", "tokens_in": 50,
                "tokens_out": 30, "turns": 3, "tampered": True,
            },
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
