"""Three-axis bench verdict — quality + tokens + wall.

Compares a fresh /bench-validate run against bench/baseline.json and
emits PASS/FAIL per axis. Exits non-zero on FAIL so the skill can gate
commits.

Library + CLI. Library callers import build_verdict + render_verdict.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def load_run(path: Path) -> list[dict[str, Any]]:
    """Read a microbench .jsonl output. Returns empty list when missing."""
    if not path.exists():
        return []
    out: list[dict[str, Any]] = []
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            out.append(json.loads(line))
    return out


def load_judge(path: Path) -> list[dict[str, Any]]:
    """Read a .judge.jsonl companion. Returns empty list when missing."""
    return load_run(path)  # same shape


def load_baseline(path: Path) -> dict[str, Any] | None:
    """Read bench/baseline.json. Returns None when missing."""
    if not path.exists():
        return None
    return json.loads(path.read_text())
