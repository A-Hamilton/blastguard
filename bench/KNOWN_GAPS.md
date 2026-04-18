# Known Gaps — resolved by Plan 8 rewrite

Gap 1 (schema mismatch) — RESOLVED. `bench/tasks.py` now loads the real
`ScaleAI/SWE-bench_Pro` schema with lowercase keys and JSON-encoded
test lists.

Gap 2 (pytest-only grading) — RESOLVED. Grading is delegated to
`scaleapi/SWE-bench_Pro-os` via `bench/evaluator.py`. Multi-language
tasks grade correctly because the evaluator uses per-repo Docker images
and per-repo test runners. We still filter to Python-only in `load_tasks`
as an MVP scope control — remove `python_only=True` to grade all
languages.

Gap 3 (no dep install) — RESOLVED. The evaluator's per-repo Docker
images include dependencies. `setup_workspace` in `runner.py` is no
longer needed; patches apply inside the evaluator container.

Gap 4 (HF_HOME) — UNCHANGED. Set `HF_HOME=/tmp/hf` before running
anything that touches the datasets library.

# Remaining deferred work (Plan 9+)

- JS / Go / Rust / Java / C++ task support — drop `python_only=True` and
  confirm evaluator runs green against non-Python repos.
- Multi-seed analysis to shrink CI further.
- Parallel per-task rollouts (currently sequential).
- Watch SWE-bench_Pro-os issues #75 (image inconsistency), #76 (test
  name mismatches), #78 (silent rate-limit failures). Our wrapper
  exposes #78 as `infra_failure` so it doesn't contaminate McNemar's.
