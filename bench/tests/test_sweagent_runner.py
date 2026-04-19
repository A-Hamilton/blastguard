"""sweagent_runner tests.

Mocks sweagent as a fake binary that writes predictable trajectory and
prediction files in the real SWE-agent v1.1.0 output format.

Real format (verified against sweagent/run/common.py + sweagent/agent/agents.py):
  <output_dir>/<instance_id>/<instance_id>.traj — JSON with info.model_stats
  <output_dir>/<instance_id>/<instance_id>.pred — JSON with model_patch

Model stats fields (from ModelStats dataclass in sweagent/agent/models.py):
  tokens_sent, tokens_received, api_calls, instance_cost
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest


def _fake_sweagent_source(instance_id: str, output: dict) -> str:
    """Return Python source for a fake sweagent binary.

    Parses --output_dir and --problem_statement.path from argv, writes
    real-format .traj and .pred files, then exits 0.
    """
    return (
        "import json, sys, pathlib\n"
        "args = sys.argv[1:]\n"
        "output_dir = None\n"
        "prob_path = None\n"
        "for i, a in enumerate(args):\n"
        "    if a == '--output_dir' and i + 1 < len(args):\n"
        "        output_dir = pathlib.Path(args[i + 1])\n"
        "    if a == '--problem_statement.path' and i + 1 < len(args):\n"
        "        prob_path = pathlib.Path(args[i + 1])\n"
        "if output_dir is None:\n"
        "    sys.stderr.write('fake_sweagent: no --output_dir\\n'); sys.exit(2)\n"
        f"iid = {instance_id!r}\n"
        "inst_dir = output_dir / iid\n"
        "inst_dir.mkdir(parents=True, exist_ok=True)\n"
        f"traj = {json.dumps({'trajectory': [], 'info': output})!r}\n"
        "(inst_dir / (iid + '.traj')).write_text(traj)\n"
        f"pred = {json.dumps({'instance_id': instance_id, 'model_name_or_path': 'fake', 'model_patch': output.get('submission', '')})!r}\n"
        "(inst_dir / (iid + '.pred')).write_text(pred)\n"
        "sys.exit(0)\n"
    )


@pytest.fixture()
def fake_sweagent_clean(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Fake sweagent that writes a clean submitted trajectory for instance demo__1."""
    traj_info = {
        "exit_status": "submitted",
        "submission": "diff --git a/foo.py b/foo.py\n+fixed\n",
        "model_stats": {
            "tokens_sent": 12345,
            "tokens_received": 678,
            "api_calls": 4,
            "instance_cost": 0.05,
        },
    }
    fake = tmp_path / "fake_sweagent.py"
    fake.write_text(_fake_sweagent_source("demo__1", traj_info))
    monkeypatch.setenv("SWEAGENT_BINARY", f"{sys.executable} {fake}")
    return fake


@pytest.fixture()
def fake_sweagent_blastguard(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    """Fake sweagent for demo__2; records argv into a sidecar file."""
    traj_info = {
        "exit_status": "submitted",
        "submission": "diff --git a/bar.py b/bar.py\n+ok\n",
        "model_stats": {
            "tokens_sent": 1000,
            "tokens_received": 200,
            "api_calls": 2,
            "instance_cost": 0.01,
        },
    }
    fake = tmp_path / "fake_sweagent_bg.py"
    # Also writes argv to <output_dir>/argv.json for test inspection.
    source = _fake_sweagent_source("demo__2", traj_info)
    source += (
        "if output_dir:\n"
        "    (output_dir / 'argv.json').write_text(json.dumps(sys.argv[1:]))\n"
    )
    fake.write_text(source)
    monkeypatch.setenv("SWEAGENT_BINARY", f"{sys.executable} {fake}")
    return fake


def _make_task(task_id: str, **kwargs):
    from bench.tasks import Task

    defaults = dict(
        repo="demo/demo",
        base_commit="abc123",
        problem_statement="fix the bug",
        fail_to_pass=[],
        pass_to_pass=[],
        language="python",
        dockerhub_tag="tag",
    )
    defaults.update(kwargs)
    return Task(task_id=task_id, **defaults)


# ---------------------------------------------------------------------------
# Test 1: happy-path patch + token parsing
# ---------------------------------------------------------------------------


def test_run_arm_returns_patch_and_token_count(
    fake_sweagent_clean, tmp_path: Path
) -> None:
    """run_arm parses tokens_sent/tokens_received/api_calls from .traj and
    model_patch from .pred into ArmResult."""
    from bench.sweagent_runner import run_arm

    task = _make_task("demo__1")
    out_dir = tmp_path / "out"

    res = run_arm(
        arm="raw",
        task=task,
        model="minimax/minimax-m2.7",
        workspace=tmp_path / "work",
        output_dir=out_dir,
    )

    assert res.patch.startswith("diff --git"), f"unexpected patch: {res.patch!r}"
    assert res.tokens.input == 12345
    assert res.tokens.output == 678
    assert res.tokens.turns == 4
    assert res.tokens.cached_input == 0  # not surfaced by SWE-agent
    assert res.exit_status == "submitted"

    # Trajectory path points at the real .traj file
    assert res.trajectory_path.exists()
    assert res.trajectory_path.name == "demo__1.traj"


# ---------------------------------------------------------------------------
# Test 2: blastguard arm generates a config YAML with the bundle path
# ---------------------------------------------------------------------------


def test_run_arm_blastguard_generates_config_with_bundle(
    fake_sweagent_blastguard, tmp_path: Path
) -> None:
    """When arm=blastguard, run_arm must write a per-invocation config YAML
    that includes the blastguard bundle under agent.tools.bundles."""
    import yaml

    from bench.sweagent_runner import BUNDLE_PATH, run_arm

    task = _make_task("demo__2")
    out_dir = tmp_path / "out-bg"

    run_arm(
        arm="blastguard",
        task=task,
        model="minimax/minimax-m2.7",
        workspace=tmp_path / "work",
        output_dir=out_dir,
    )

    # The runner must write a config file into out_dir for reproducibility.
    config_files = list(out_dir.glob("*.yaml"))
    assert config_files, f"No YAML config written to {out_dir}. Files: {list(out_dir.iterdir())}"

    config_yaml = config_files[0].read_text()
    config = yaml.safe_load(config_yaml)

    bundles = config.get("agent", {}).get("tools", {}).get("bundles", [])
    bundle_paths = [b.get("path", "") if isinstance(b, dict) else str(b) for b in bundles]
    assert any(str(BUNDLE_PATH) in str(p) for p in bundle_paths), (
        f"BlastGuard bundle path not found in config bundles: {bundle_paths}\n"
        f"Expected to contain: {BUNDLE_PATH}"
    )


# ---------------------------------------------------------------------------
# Test 3: rate-limit retry
# ---------------------------------------------------------------------------


def test_run_arm_retries_once_on_rate_limit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """First invocation exits non-zero with 'RateLimitError' in stderr;
    second invocation succeeds and writes trajectory + pred."""
    counter_file = tmp_path / "count.txt"
    counter_file.write_text("0")

    # The fake writes trajectories on the SECOND call only.
    traj_info = {
        "exit_status": "submitted",
        "submission": "diff --git a/x.py b/x.py\n+rate-retry\n",
        "model_stats": {"tokens_sent": 1, "tokens_received": 1, "api_calls": 1},
    }
    fake = tmp_path / "fake_sweagent_rl.py"
    fake.write_text(
        "import json, sys, pathlib, os\n"
        f"cf = pathlib.Path({str(counter_file)!r})\n"
        "n = int(cf.read_text()); cf.write_text(str(n + 1))\n"
        "if n == 0:\n"
        "    sys.stderr.write('RateLimitError: 429 Too Many Requests')\n"
        "    sys.exit(1)\n"
        + _fake_sweagent_source("demo__rl", traj_info)
    )
    monkeypatch.setenv("SWEAGENT_BINARY", f"{sys.executable} {fake}")
    monkeypatch.setenv("BENCH_RATE_LIMIT_SLEEP", "0")  # skip real sleep in tests

    from bench.sweagent_runner import run_arm

    task = _make_task("demo__rl")
    out_dir = tmp_path / "out-rl"

    res = run_arm(
        arm="raw",
        task=task,
        model="minimax/minimax-m2.7",
        workspace=tmp_path / "work",
        output_dir=out_dir,
    )

    assert res.patch.startswith("diff"), f"unexpected patch: {res.patch!r}"
    assert int(counter_file.read_text()) == 2, "expected exactly 2 sweagent invocations"
