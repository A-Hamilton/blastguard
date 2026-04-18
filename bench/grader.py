"""Grade a patch application against the task's fail-to-pass tests.

SPEC §15.4: BenchJack defense. An agent can silently ship a 10-line
`conftest.py` that forces pytest to report all tests as passing.
Countermeasures:

1. Before grading, diff agent-touched files against the base commit.
   Any change to `conftest.py`, `pytest.ini`, `pyproject.toml`
   (`[tool.pytest*]` only), `tox.ini`, or `.github/workflows/**`
   is treated as tampering.
2. Run pytest in a subprocess with `-p no:cacheprovider -o addopts=`
   so user/global config can't alter test selection.
3. Honour the task's exact test node-ids; no `-k` or `--tb` overrides
   that could widen the selection.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path

TAMPER_PATTERNS: tuple[str, ...] = (
    "conftest.py",
    "pytest.ini",
    "tox.ini",
    # pyproject.toml can embed [tool.pytest.ini_options] — an agent rewriting
    # that section can skip or re-select tests. The BenchJack exploit class
    # includes this vector, so any pyproject.toml change is flagged. A smarter
    # filter would diff the [tool.pytest*] sections only, but for SWE-bench
    # Pro grading the agent has no reason to touch pyproject.toml at all.
    "pyproject.toml",
    "setup.cfg",  # also accepts [tool:pytest]
)


@dataclass(frozen=True, slots=True)
class GradeResult:
    resolved: bool
    tampered: bool
    tampered_files: tuple[str, ...]
    fail_to_pass_passed: int
    fail_to_pass_total: int
    pass_to_pass_passed: int
    pass_to_pass_total: int
    stderr: str


def detect_tampering(
    workspace: Path,
    changed_files: list[str],
) -> tuple[bool, tuple[str, ...]]:
    """Return (tampered, files_that_tampered).

    `changed_files` is the list of paths the agent modified relative to
    `workspace`. Any match against `TAMPER_PATTERNS` triggers tampering.
    """
    tampered: list[str] = []
    for path in changed_files:
        rel = path.lstrip("/")
        for pattern in TAMPER_PATTERNS:
            if rel.endswith(pattern) or rel == pattern:
                tampered.append(rel)
                break
        # GitHub workflow files — an agent rewriting CI is also tampering.
        if rel.startswith(".github/workflows/"):
            tampered.append(rel)
    return (len(tampered) > 0, tuple(tampered))


def grade(
    workspace: Path,
    changed_files: list[str],
    fail_to_pass: list[str],
    pass_to_pass: list[str],
) -> GradeResult:
    """Run pytest against the specified test node-ids and classify."""
    tampered, tampered_files = detect_tampering(workspace, changed_files)
    if tampered:
        return GradeResult(
            resolved=False,
            tampered=True,
            tampered_files=tampered_files,
            fail_to_pass_passed=0,
            fail_to_pass_total=len(fail_to_pass),
            pass_to_pass_passed=0,
            pass_to_pass_total=len(pass_to_pass),
            stderr=f"tampering detected: {', '.join(tampered_files)}",
        )

    if not fail_to_pass and not pass_to_pass:
        return GradeResult(
            resolved=False,
            tampered=False,
            tampered_files=(),
            fail_to_pass_passed=0,
            fail_to_pass_total=0,
            pass_to_pass_passed=0,
            pass_to_pass_total=0,
            stderr="no tests to grade",
        )

    # Build a pytest command that can't be overridden by local config.
    args = [
        "python",
        "-m",
        "pytest",
        "-p",
        "no:cacheprovider",
        "-o",
        "addopts=",
        "--no-header",
        "--tb=short",
        "-q",
        *fail_to_pass,
        *pass_to_pass,
    ]
    completed = subprocess.run(
        args,
        cwd=workspace,
        capture_output=True,
        text=True,
        timeout=900,
        check=False,
    )
    ftp_passed, ptp_passed = _count_passes(completed.stdout, fail_to_pass, pass_to_pass)
    resolved = (
        ftp_passed == len(fail_to_pass)
        and ptp_passed == len(pass_to_pass)
        and len(fail_to_pass) > 0
    )
    return GradeResult(
        resolved=resolved,
        tampered=False,
        tampered_files=(),
        fail_to_pass_passed=ftp_passed,
        fail_to_pass_total=len(fail_to_pass),
        pass_to_pass_passed=ptp_passed,
        pass_to_pass_total=len(pass_to_pass),
        stderr=completed.stderr[:500],
    )


def _count_passes(
    stdout: str,
    fail_to_pass: list[str],
    pass_to_pass: list[str],
) -> tuple[int, int]:
    """Scan pytest -q output for 'PASSED <nodeid>' lines.

    pytest -q doesn't emit per-test PASSED by default; fall back to
    summary parsing. A robust implementation uses `--json-report` but
    for Phase 1 we keep the dependency surface minimal.
    """
    ftp_passed = 0
    ptp_passed = 0
    # Simple heuristic: pytest -q prints "N passed" in the summary line.
    # For per-test attribution we scan for "PASSED " prefixes when
    # -rA or --tb=short surface them.
    passed_lines = {line.strip() for line in stdout.splitlines() if "PASSED" in line}
    for node in fail_to_pass:
        if any(node in line for line in passed_lines):
            ftp_passed += 1
    for node in pass_to_pass:
        if any(node in line for line in passed_lines):
            ptp_passed += 1
    # Fallback: if no per-test PASSED lines, trust the summary — if
    # "N passed, 0 failed" and N >= expected, count everything passed.
    if ftp_passed == 0 and ptp_passed == 0:
        for line in stdout.splitlines():
            if " passed" in line:
                # "1 passed, 0 failed in 0.5s"
                parts = line.split()
                try:
                    n_passed = int(parts[0])
                    n_failed_idx = parts.index("failed,") if "failed," in parts else None
                    if n_failed_idx is None:
                        # "3 passed in 0.5s"
                        if n_passed >= len(fail_to_pass) + len(pass_to_pass):
                            return (len(fail_to_pass), len(pass_to_pass))
                    else:
                        n_failed = int(parts[n_failed_idx - 1])
                        if n_failed == 0 and n_passed >= len(fail_to_pass) + len(pass_to_pass):
                            return (len(fail_to_pass), len(pass_to_pass))
                except (ValueError, IndexError):
                    pass
    return (ftp_passed, ptp_passed)
