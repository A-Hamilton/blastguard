"""Grade a patch application against the task's fail-to-pass tests.

SPEC §15.4: BenchJack defense. An agent can silently ship a 10-line
`conftest.py` that forces pytest to report all tests as passing.
Countermeasures:

1. Before grading, diff agent-touched files against the base commit.
   Any change to `conftest.py`, `pytest.ini`, `pyproject.toml`
   (`[tool.pytest*]` only), `tox.ini`, or `.github/workflows/**`
   is treated as tampering.
2. Grading is now delegated to scaleapi/SWE-bench_Pro-os via
   `bench/evaluator.py`. The old pytest-based `grade()` function
   has been removed — see evaluator.py for the replacement.
3. `detect_tampering` remains and is called by the runner before
   submitting a patch to the evaluator.
"""

from __future__ import annotations

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


def detect_tampering(
    changed_files: list[str],
) -> tuple[bool, tuple[str, ...]]:
    """Return (tampered, files_that_tampered).

    `changed_files` is the list of paths the agent modified relative to
    the workspace root. Any match against `TAMPER_PATTERNS` triggers
    tampering. GitHub workflow changes are also treated as tampering.
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
