"""Wrapper for scaleapi/SWE-bench_Pro-os evaluator.

We shell out to `swe_bench_pro_eval.py`. The evaluator's per-instance
output JSON is our only source of truth for pass/fail. Issue #78 (open)
means rate-limit/infra errors inside the evaluator can be silently
scored as task failures; we detect this by checking for an `error` key
or missing `resolved` field and flag those as `infra_failure=True` so
`compare.py` can exclude them from McNemar's counts.
"""

from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class EvaluatorResult:
    task_id: str
    resolved: bool
    infra_failure: bool
    raw: dict


def write_patches_json(
    predictions: list[tuple[str, str]],
    prefix: str,
    out_path: Path,
) -> None:
    """Emit the JSON array format expected by the evaluator.

    predictions: list of (instance_id, unified_diff_patch) tuples.
    """
    data = [
        {"instance_id": iid, "patch": patch, "prefix": prefix}
        for iid, patch in predictions
    ]
    out_path.write_text(json.dumps(data, indent=2))


def parse_evaluator_output(out_dir: Path) -> list[EvaluatorResult]:
    """Read every *.json in out_dir and classify it."""
    results: list[EvaluatorResult] = []
    for path in sorted(out_dir.glob("*.json")):
        try:
            payload = json.loads(path.read_text())
        except json.JSONDecodeError:
            results.append(
                EvaluatorResult(
                    task_id=path.stem,
                    resolved=False,
                    infra_failure=True,
                    raw={"error": "json_decode_error", "file": path.name},
                )
            )
            continue

        task_id = str(payload.get("instance_id", path.stem))
        has_error = bool(payload.get("error")) or "resolved" not in payload
        resolved = bool(payload.get("resolved", False)) if not has_error else False

        results.append(
            EvaluatorResult(
                task_id=task_id,
                resolved=resolved,
                infra_failure=has_error,
                raw=payload,
            )
        )
    return results


def run_evaluator(
    *,
    evaluator_dir: Path,
    raw_sample_csv: Path,
    patches_json: Path,
    output_dir: Path,
    num_workers: int = 4,
    dockerhub_username: str = "jefzda",
    timeout_seconds: int = 3600,
) -> int:
    """Invoke the evaluator as a subprocess. Returns its exit code."""
    output_dir.mkdir(parents=True, exist_ok=True)
    cmd = [
        "python",
        str(evaluator_dir / "swe_bench_pro_eval.py"),
        f"--raw_sample_path={raw_sample_csv}",
        f"--patch_path={patches_json}",
        f"--output_dir={output_dir}",
        f"--scripts_dir={evaluator_dir / 'run_scripts'}",
        f"--num_workers={num_workers}",
        f"--dockerhub_username={dockerhub_username}",
    ]
    proc = subprocess.run(
        cmd,
        cwd=evaluator_dir,
        timeout=timeout_seconds,
        check=False,
    )
    return proc.returncode
