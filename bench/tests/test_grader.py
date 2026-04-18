"""Unit tests for the grader — focus on tamper detection."""

from __future__ import annotations

from pathlib import Path

import pytest

from bench.grader import detect_tampering, grade


def test_conftest_change_is_tampering() -> None:
    tampered, files = detect_tampering(Path("/tmp"), ["src/foo.py", "tests/conftest.py"])
    assert tampered is True
    assert "tests/conftest.py" in files


def test_pytest_ini_change_is_tampering() -> None:
    tampered, files = detect_tampering(Path("/tmp"), ["pytest.ini"])
    assert tampered
    assert files == ("pytest.ini",)


def test_github_workflow_change_is_tampering() -> None:
    tampered, files = detect_tampering(
        Path("/tmp"),
        [".github/workflows/ci.yml"],
    )
    assert tampered
    assert ".github/workflows/ci.yml" in files


def test_source_change_is_not_tampering() -> None:
    tampered, files = detect_tampering(Path("/tmp"), ["src/foo.py"])
    assert tampered is False
    assert files == ()


def test_grade_returns_tampered_when_conftest_touched(tmp_path: Path) -> None:
    result = grade(
        workspace=tmp_path,
        changed_files=["conftest.py"],
        fail_to_pass=["tests.x::test_y"],
        pass_to_pass=[],
    )
    assert result.tampered is True
    assert result.resolved is False
    assert "tampering detected" in result.stderr


def test_grade_empty_tests_returns_unresolved(tmp_path: Path) -> None:
    result = grade(workspace=tmp_path, changed_files=[], fail_to_pass=[], pass_to_pass=[])
    assert result.resolved is False
    assert result.tampered is False


@pytest.mark.skipif(
    not (__import__("shutil").which("pytest")), reason="pytest not on PATH"
)
def test_grade_passing_test_reports_resolved(tmp_path: Path) -> None:
    """Seed a trivial passing pytest module + run grade()."""
    # Create a minimal test file.
    (tmp_path / "tests").mkdir()
    (tmp_path / "tests" / "__init__.py").write_text("")
    (tmp_path / "tests" / "test_simple.py").write_text(
        "def test_one():\n    assert 1 + 1 == 2\n"
    )
    result = grade(
        workspace=tmp_path,
        changed_files=["tests/test_simple.py"],
        fail_to_pass=["tests/test_simple.py::test_one"],
        pass_to_pass=[],
    )
    # tests/test_simple.py is a source file, not a pytest config file,
    # so no tampering.
    assert result.tampered is False
    # The test passes under real pytest.
    assert result.resolved is True, f"expected resolved, got {result}"
