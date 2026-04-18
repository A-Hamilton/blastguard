"""Paired analysis reporter.

Loads two arms' evaluator outputs, pairs them by task_id, excludes any
task where either arm hit an infra_failure (rate limits, Docker crashes,
evaluator errors), then runs McNemar's test.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from bench.evaluator import EvaluatorResult, parse_evaluator_output
from bench.stats import mcnemar_paired


def pair_results(
    raw: list[EvaluatorResult],
    blastguard: list[EvaluatorResult],
) -> dict[str, tuple[EvaluatorResult, EvaluatorResult]]:
    """Intersect by task_id and drop infra failures from either arm."""
    raw_map = {r.task_id: r for r in raw if not r.infra_failure}
    bg_map = {r.task_id: r for r in blastguard if not r.infra_failure}
    shared = raw_map.keys() & bg_map.keys()
    return {tid: (raw_map[tid], bg_map[tid]) for tid in shared}


def format_report(
    raw: list[EvaluatorResult],
    blastguard: list[EvaluatorResult],
) -> str:
    pairs = pair_results(raw, blastguard)
    both_pass = both_fail = raw_only = bg_only = 0
    for r, b in pairs.values():
        if r.resolved and b.resolved:
            both_pass += 1
        elif not r.resolved and not b.resolved:
            both_fail += 1
        elif r.resolved and not b.resolved:
            raw_only += 1
        else:
            bg_only += 1

    stats = mcnemar_paired([
        ("both_pass", both_pass),
        ("both_fail", both_fail),
        ("raw_only_pass", raw_only),
        ("blastguard_only_pass", bg_only),
    ])

    raw_infra = sum(1 for r in raw if r.infra_failure)
    bg_infra = sum(1 for r in blastguard if r.infra_failure)

    return (
        f"Paired McNemar's Test — BlastGuard vs raw\n"
        f"===========================================\n"
        f"Paired tasks:          {stats.n}\n"
        f"Infra failures (raw):  {raw_infra} (excluded)\n"
        f"Infra failures (bg):   {bg_infra} (excluded)\n"
        f"\n"
        f"Both pass:             {stats.both_pass}\n"
        f"Both fail:             {stats.both_fail}\n"
        f"Raw wins (only raw):   {stats.raw_wins}\n"
        f"BlastGuard wins:       {stats.blastguard_wins}\n"
        f"\n"
        f"Raw score:             {stats.raw_score_pct:.2f}%\n"
        f"BlastGuard score:      {stats.blastguard_score_pct:.2f}%\n"
        f"Delta:                 {stats.delta_pct:+.2f} pp\n"
        f"\n"
        f"Test:                  {stats.test_used}\n"
        f"p-value:               {stats.p_value:.4f}\n"
        f"Significant (α=0.05):  {'YES' if stats.p_value < 0.05 else 'NO'}\n"
    )


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--raw-output-dir", type=Path, required=True)
    p.add_argument("--blastguard-output-dir", type=Path, required=True)
    args = p.parse_args()

    raw = parse_evaluator_output(args.raw_output_dir)
    bg = parse_evaluator_output(args.blastguard_output_dir)
    print(format_report(raw, bg))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
