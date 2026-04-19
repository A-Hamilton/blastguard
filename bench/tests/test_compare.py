"""Unit tests for compare.py paired analysis reporter."""

from __future__ import annotations

from bench.compare import format_report, pair_results
from bench.evaluator import EvaluatorResult


def _res(task_id: str, resolved: bool, infra_failure: bool = False) -> EvaluatorResult:
    return EvaluatorResult(task_id=task_id, resolved=resolved, infra_failure=infra_failure, raw={})


def test_pair_results_excludes_infra_failures_from_either_arm():
    raw = [_res("a", True), _res("b", False), _res("c", False, infra_failure=True)]
    bg = [_res("a", True), _res("b", True), _res("c", True)]
    paired = pair_results(raw, bg)
    # "c" is excluded because raw hit an infra failure
    assert set(paired.keys()) == {"a", "b"}


def test_format_report_includes_mcnemar():
    raw = [_res("a", True), _res("b", False), _res("c", False), _res("d", True)]
    bg = [_res("a", True), _res("b", True), _res("c", True), _res("d", False)]
    report = format_report(raw, bg)
    assert "McNemar" in report
    assert "blastguard_wins" in report.lower() or "BlastGuard wins" in report
