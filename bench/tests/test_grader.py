"""Unit tests for the grader — focus on tamper detection.

grade() and _count_passes() were removed in Plan 8; grading is now
handled by bench/evaluator.py (SWE-bench_Pro-os subprocess wrapper).
"""

from __future__ import annotations

from bench.grader import detect_tampering


def test_conftest_change_is_tampering() -> None:
    tampered, files = detect_tampering(["src/foo.py", "tests/conftest.py"])
    assert tampered is True
    assert "tests/conftest.py" in files


def test_pytest_ini_change_is_tampering() -> None:
    tampered, files = detect_tampering(["pytest.ini"])
    assert tampered
    assert files == ("pytest.ini",)


def test_github_workflow_change_is_tampering() -> None:
    tampered, files = detect_tampering([".github/workflows/ci.yml"])
    assert tampered
    assert ".github/workflows/ci.yml" in files


def test_source_change_is_not_tampering() -> None:
    tampered, files = detect_tampering(["src/foo.py"])
    assert tampered is False
    assert files == ()


def test_empty_changed_files_is_not_tampering() -> None:
    tampered, files = detect_tampering([])
    assert tampered is False
    assert files == ()
