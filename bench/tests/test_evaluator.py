from bench.evaluator import (
    EvaluatorResult,
    parse_evaluator_output,
    write_patches_json,
)
from pathlib import Path
import json


def test_parse_evaluator_output_resolved(tmp_path: Path):
    """A resolved task maps to EvaluatorResult(resolved=True, infra_failure=False)."""
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "django__123.json").write_text(
        json.dumps({
            "instance_id": "django__123",
            "resolved": True,
            "tests_status": {"fail_to_pass": {"success": ["t1"], "failure": []}},
        })
    )
    results = parse_evaluator_output(out_dir)
    assert len(results) == 1
    r = results[0]
    assert r.task_id == "django__123"
    assert r.resolved is True
    assert r.infra_failure is False


def test_parse_evaluator_output_infra_failure(tmp_path: Path):
    """An empty / errored result is flagged as infra_failure, not a clean fail."""
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "django__456.json").write_text(
        json.dumps({
            "instance_id": "django__456",
            "error": "rate_limit",
        })
    )
    results = parse_evaluator_output(out_dir)
    r = results[0]
    assert r.resolved is False
    assert r.infra_failure is True


def test_write_patches_json_matches_evaluator_format(tmp_path: Path):
    """The evaluator expects a JSON array of {instance_id, patch, prefix}."""
    out = tmp_path / "patches.json"
    write_patches_json(
        [("django__1", "diff --git a b\n+ foo"), ("sklearn__2", "")],
        prefix="blastguard-m27",
        out_path=out,
    )
    data = json.loads(out.read_text())
    assert isinstance(data, list)
    assert data[0]["instance_id"] == "django__1"
    assert data[0]["patch"].startswith("diff --git")
    assert data[0]["prefix"] == "blastguard-m27"
    assert data[1]["patch"] == ""
