"""Deterministic microbench grader — Priority 1a quality measurement.

Reads a `.jsonl` run file emitted by `bench/microbench.py` and scores
each rollout `correct=True` when every `expected_substring` for that
task appears (case-insensitive) in the rollout's `final_answer`.

Aggregates into a `correctness_rate` per (task, arm) so
`bench-regression-guard` can block commits where BG's correctness
drops below raw's within a tolerance (default 2 percentage points).

This is Priority 1a: cheap, reproducible, catches hard regressions.
Priority 1b (LLM-as-judge via a second Gemma instance doing blind
pairwise ranking of arm outputs) is a follow-on — it judges fluency
and subtle hallucination where substring matching is too loose.

Priority 2 (token deltas) and Priority 3 (wall time) are already
covered by `bench/stats_aggregate.py`.

Library-only — no CLI. Mirrors the `stats_aggregate.py` convention.
Callers:
    from bench.microbench_grader import grade_rollouts, correctness_rate_by_cell
    graded = grade_rollouts(runs, TASKS)
    rates = correctness_rate_by_cell(graded)
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass
class GradedRollout:
    """A rollout plus its deterministic correctness judgment."""

    task_id: str
    arm: str
    seed: int
    correct: bool
    missing_substrings: list[str]
    stopped_reason: str
    final_answer: str


def grade_rollouts(
    runs: list[dict[str, Any]],
    tasks: list[dict[str, Any]],
) -> list[GradedRollout]:
    """Score every rollout against its task's expected substrings.

    A rollout is correct iff every `expected_substring` for its task
    appears (case-insensitive) in `final_answer`. Tasks without
    `expected_substrings` are skipped silently — not all tasks in the
    registry need a grading rubric.
    """
    task_index = {t["id"]: t for t in tasks}
    graded: list[GradedRollout] = []
    for r in runs:
        task = task_index.get(r.get("task_id") or r.get("task"))
        if task is None:
            continue
        expected = task.get("expected_substrings")
        if not expected:
            continue
        answer_lower = (r.get("final_answer") or "").lower()
        missing = [s for s in expected if s.lower() not in answer_lower]
        graded.append(
            GradedRollout(
                task_id=task["id"],
                arm=r.get("arm", "unknown"),
                seed=int(r.get("seed", 0)),
                correct=not missing,
                missing_substrings=missing,
                stopped_reason=r.get("stopped_reason", ""),
                final_answer=r.get("final_answer", ""),
            )
        )
    return graded


def correctness_rate_by_cell(
    graded: list[GradedRollout],
) -> dict[tuple[str, str], dict[str, Any]]:
    """Aggregate correctness rate per (task_id, arm) cell.

    Each cell holds:
    - `n`: number of graded rollouts
    - `correct`: count of correct rollouts
    - `correct_rate`: float in [0, 1]
    - `missing`: list of unique `missing_substrings` across failed rollouts
      (useful for diagnosing what the arm consistently omits)
    """
    cells: dict[tuple[str, str], dict[str, Any]] = {}
    for g in graded:
        key = (g.task_id, g.arm)
        cell = cells.setdefault(
            key,
            {"n": 0, "correct": 0, "correct_rate": 0.0, "missing": set()},
        )
        cell["n"] += 1
        if g.correct:
            cell["correct"] += 1
        else:
            cell["missing"].update(g.missing_substrings)
    for cell in cells.values():
        if cell["n"] > 0:
            cell["correct_rate"] = cell["correct"] / cell["n"]
        cell["missing"] = sorted(cell["missing"])
    return cells


def regression_verdict(
    cells: dict[tuple[str, str], dict[str, Any]],
    tolerance_pp: float = 2.0,
) -> tuple[str, list[str]]:
    """Compare BG correctness to raw per task. Returns (verdict, reasons).

    - `COMMIT OK` when every task's BG rate is within `tolerance_pp`
      percentage points of the raw rate (including BG >= raw cases).
    - `DO NOT COMMIT` when any task's BG rate drops more than
      `tolerance_pp` percentage points below raw.

    A missing cell for either arm is a warning, not a blocker — it
    just means the bench didn't run that (task, arm) pair.
    """
    tasks = sorted({task_id for (task_id, _) in cells})
    reasons: list[str] = []
    regressed = False
    for task_id in tasks:
        raw = cells.get((task_id, "raw"))
        bg = cells.get((task_id, "blastguard"))
        if raw is None or bg is None:
            reasons.append(f"{task_id}: missing arm data (raw={raw is not None}, bg={bg is not None})")
            continue
        raw_rate = float(raw["correct_rate"])
        bg_rate = float(bg["correct_rate"])
        diff_pp = (bg_rate - raw_rate) * 100.0
        if diff_pp < -tolerance_pp:
            regressed = True
            reasons.append(
                f"{task_id}: BG {bg_rate:.0%} vs raw {raw_rate:.0%} "
                f"(Δ {diff_pp:+.1f}pp, beyond ±{tolerance_pp:.1f}pp tolerance)"
            )
        else:
            reasons.append(
                f"{task_id}: BG {bg_rate:.0%} vs raw {raw_rate:.0%} "
                f"(Δ {diff_pp:+.1f}pp)"
            )
    verdict = "DO NOT COMMIT" if regressed else "COMMIT OK"
    return verdict, reasons
