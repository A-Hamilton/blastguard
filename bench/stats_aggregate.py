"""Multi-seed / multi-task micro-bench aggregation.

Reads one or more JSONL files (each line is a `RunResult` as emitted by
`bench/microbench.py`) and produces:

- `load_runs(paths)` -> `list[dict]` — flat list of records from all files
- `aggregate_per_cell(runs)` -> `dict[(task_id, arm), metrics]` — mean,
  std, n, and min/max for cost / input_tokens / turns / wall_seconds
- `arm_totals_with_ci(runs)` -> aggregate per arm with a paired-difference
  95% CI on `(raw − blastguard)` cost. If only one seed is present the CI
  width will be NaN and downstream consumers should treat that row as
  "single draw, no variance estimate".

Uses only stdlib + `statistics` for means/std and `scipy.stats.t` for
the paired CI (already a bench dep via `bench/pyproject.toml`).
"""

from __future__ import annotations

import json
import math
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any

from scipy.stats import t as student_t


def load_runs(paths: list[Path]) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for path in paths:
        with Path(path).open("r", encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                out.append(json.loads(line))
    return out


def _population_std(values: list[float]) -> float:
    # Population std matches numpy/statistics defaults for small n; fine
    # because we're descriptive, not inferential at the cell level.
    if len(values) < 2:
        return 0.0
    return statistics.pstdev(values)


def aggregate_per_cell(
    runs: list[dict[str, Any]],
) -> dict[tuple[str, str], dict[str, float | int]]:
    groups: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for r in runs:
        groups[(r["task_id"], r["arm"])].append(r)

    out: dict[tuple[str, str], dict[str, float | int]] = {}
    for (task_id, arm), rows in groups.items():
        costs = [r["total_cost_usd"] for r in rows]
        ins = [r["input_tokens"] for r in rows]
        turns = [r["turns"] for r in rows]
        walls = [r["wall_seconds"] for r in rows]
        out[(task_id, arm)] = {
            "n": len(rows),
            "cost_mean": statistics.fmean(costs),
            "cost_std": _population_std(costs),
            "cost_min": min(costs),
            "cost_max": max(costs),
            "input_mean": statistics.fmean(ins),
            "turns_mean": statistics.fmean(turns),
            "wall_mean": statistics.fmean(walls),
        }
    return out


def arm_totals_with_ci(runs: list[dict[str, Any]]) -> dict[str, Any]:
    """Per-arm totals across (task, seed) combinations + paired-difference CI.

    The paired structure is: for each (task_id, seed), raw and blastguard
    arms should both have a run. We sum each arm's cost across tasks for
    each seed, then compute a paired t-CI on (raw_total − bg_total) across
    seeds. When n_seeds == 1, the CI width is NaN and we warn.
    """
    # Index by (task_id, seed) -> {arm: cost_dict}
    by_key: dict[tuple[str, int], dict[str, dict]] = defaultdict(dict)
    for r in runs:
        by_key[(r["task_id"], r["seed"])][r["arm"]] = r

    # Per-seed totals.
    seeds = sorted({seed for (_, seed) in by_key})
    per_seed_totals: dict[str, dict[int, float]] = {"raw": {}, "blastguard": {}}
    for seed in seeds:
        raw_cost = 0.0
        bg_cost = 0.0
        for (_task_id, s), arms in by_key.items():
            if s != seed:
                continue
            if "raw" in arms:
                raw_cost += arms["raw"]["total_cost_usd"]
            if "blastguard" in arms:
                bg_cost += arms["blastguard"]["total_cost_usd"]
        per_seed_totals["raw"][seed] = raw_cost
        per_seed_totals["blastguard"][seed] = bg_cost

    raw_costs = list(per_seed_totals["raw"].values())
    bg_costs = list(per_seed_totals["blastguard"].values())

    out: dict[str, Any] = {
        "seeds": seeds,
        "n_seeds": len(seeds),
        "raw": {
            "cost_mean": statistics.fmean(raw_costs) if raw_costs else 0.0,
            "cost_std": _population_std(raw_costs),
        },
        "blastguard": {
            "cost_mean": statistics.fmean(bg_costs) if bg_costs else 0.0,
            "cost_std": _population_std(bg_costs),
        },
    }

    if len(seeds) < 2:
        out["paired_diff"] = {
            "mean": (out["raw"]["cost_mean"] - out["blastguard"]["cost_mean"]),
            "ci95_low": float("nan"),
            "ci95_high": float("nan"),
            "note": "single seed — no variance estimate available",
        }
        return out

    diffs = [per_seed_totals["raw"][s] - per_seed_totals["blastguard"][s] for s in seeds]
    n = len(diffs)
    mean = statistics.fmean(diffs)
    sd = statistics.stdev(diffs)  # sample std for inference
    t_crit = student_t.ppf(0.975, df=n - 1)
    half = t_crit * sd / math.sqrt(n)
    out["paired_diff"] = {
        "mean": mean,
        "ci95_low": mean - half,
        "ci95_high": mean + half,
        "per_seed_diffs": diffs,
    }
    return out


def render_markdown_report(runs: list[dict[str, Any]]) -> str:
    """Convenience wrapper that emits the Markdown section we paste into
    `docs/MICROBENCH.md` after a run.
    """
    cells = aggregate_per_cell(runs)
    totals = arm_totals_with_ci(runs)
    lines: list[str] = []
    lines.append("### Per-task means across seeds\n")
    lines.append("| task | arm | n | cost mean | cost std | turns mean | wall mean |")
    lines.append("|---|---|--:|--:|--:|--:|--:|")
    for (task_id, arm), c in sorted(cells.items()):
        lines.append(
            f"| {task_id} | {arm} | {c['n']} | "
            f"${c['cost_mean']:.4f} | ${c['cost_std']:.4f} | "
            f"{c['turns_mean']:.1f} | {c['wall_mean']:.1f}s |"
        )
    lines.append("")
    lines.append("### Arm totals with paired 95% CI on cost difference")
    lines.append("")
    lines.append(f"- seeds run: {totals['seeds']}")
    lines.append(f"- raw arm total cost (mean across seeds): ${totals['raw']['cost_mean']:.4f} "
                 f"(std ${totals['raw']['cost_std']:.4f})")
    lines.append(f"- BG arm total cost (mean across seeds): ${totals['blastguard']['cost_mean']:.4f} "
                 f"(std ${totals['blastguard']['cost_std']:.4f})")
    pd = totals["paired_diff"]
    low = pd.get("ci95_low")
    high = pd.get("ci95_high")
    if low is not None and not math.isnan(low):
        lines.append(
            f"- paired (raw − BG) mean: ${pd['mean']:.4f}, "
            f"95% CI [${low:.4f}, ${high:.4f}]"
        )
        if low > 0:
            lines.append("  **BG is cheaper than raw at 95% confidence.**")
        elif high < 0:
            lines.append("  **BG is more expensive than raw at 95% confidence.**")
        else:
            lines.append("  CI crosses zero — no statistically significant difference.")
    else:
        lines.append(
            f"- paired (raw − BG) mean: ${pd['mean']:.4f} "
            f"({pd.get('note', 'single seed')})"
        )
    return "\n".join(lines)
