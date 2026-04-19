"""Orchestrate one SWE-agent subprocess invocation per task per arm.

SWE-agent writes per-instance output to:
  <output_dir>/<instance_id>/<instance_id>.traj   — JSON with info.model_stats
  <output_dir>/<instance_id>/<instance_id>.pred   — JSON with model_patch

Field names (verified against sweagent/agent/models.py ModelStats dataclass):
  info.model_stats.tokens_sent       → TokenCount.input
  info.model_stats.tokens_received   → TokenCount.output
  info.model_stats.api_calls         → TokenCount.turns
  info.exit_status                   → ArmResult.exit_status
  pred.model_patch                   → ArmResult.patch  (null → "")

Bundle injection uses a per-invocation config YAML (written to output_dir)
because SWE-agent has no --tools.bundles CLI flag — bundles live in YAML only.
Temperature and model config also live in the YAML.

The caller (bench/runner.py) is responsible for cloning the repo and checking
out task.base_commit BEFORE calling run_arm. This module does not clone.
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

from bench.tasks import Task
from bench.token_count import TokenCount

REPO_ROOT = Path(__file__).resolve().parent.parent
BUNDLE_PATH = REPO_ROOT / "bench" / "bundles" / "blastguard"

# SWE-agent config shipped with the repo clone (provides tool registry etc.)
_SWEAGENT_REPO = Path(__file__).resolve().parent / ".sweagent-repo"
_DEFAULT_BASE_CONFIG = _SWEAGENT_REPO / "config" / "default.yaml"


@dataclass(frozen=True, slots=True)
class ArmResult:
    """Result of a single SWE-agent arm invocation."""

    patch: str
    tokens: TokenCount
    trajectory_path: Path
    exit_status: str


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _sweagent_cmd() -> list[str]:
    """Return the sweagent command list.

    Reads SWEAGENT_BINARY from env so tests can inject a fake binary without
    touching the real sweagent install.
    """
    override = os.environ.get("SWEAGENT_BINARY")
    if override:
        return shlex.split(override)
    return ["sweagent", "run"]


def _build_config_yaml(
    *,
    arm: str,
    model: str,
    api_key_env: str = "OPENROUTER_API_KEY",
    api_base: str = "https://openrouter.ai/api/v1",
    per_instance_cost_limit: float = 5.0,
    output_dir: Path,
) -> Path:
    """Write a per-invocation SWE-agent config YAML and return its path.

    The config lives in output_dir so it travels with the trajectory for
    reproducibility.  arm=blastguard adds the BlastGuard bundle to
    agent.tools.bundles; arm=raw does not.
    """
    bundles: list[dict[str, Any]] = []

    # Include the default SWE-agent tool registry if the repo is present.
    registry = _SWEAGENT_REPO / "tools" / "registry"
    if registry.exists():
        bundles.append({"path": str(registry)})

    if arm == "blastguard":
        bundles.append({"path": str(BUNDLE_PATH)})

    # LiteLLM's cost tracking covers paid models but not free-tier or unlisted
    # models. When the model isn't in its price table, SWE-agent raises
    # ModelConfigurationError unless both cost limits are 0 (disabling the
    # safety check). Free-tier models (`:free` suffix) have $0 spend by
    # construction — override both caps to 0 so the harness proceeds.
    is_free_tier = model.endswith(":free")
    effective_per_instance_limit = 0.0 if is_free_tier else per_instance_cost_limit
    effective_total_limit = 0.0

    # System template carries arm-specific steering (BLASTGUARD_BIAS on BG arm).
    # instance_template injects the SWE-bench problem statement via {{problem_statement}}.
    # Without these, the agent receives empty prompts and makes nonsense tool
    # calls until SWE-agent exits on format errors.
    from bench.prompts import build_system_prompt  # noqa: PLC0415

    system_template = build_system_prompt(arm=arm)
    instance_template = (
        "<uploaded_files>\n"
        "{{working_dir}}\n"
        "</uploaded_files>\n"
        "Consider the following SWE-bench Pro task:\n\n"
        "<task>\n"
        "{{problem_statement}}\n"
        "</task>\n\n"
        "Make the minimal edits required to make the fail-to-pass tests "
        "pass without breaking any pass-to-pass tests. When your edit is "
        "complete, call the submit command."
    )
    next_step_template = "OBSERVATION:\n{{observation}}"

    config: dict[str, Any] = {
        "agent": {
            "templates": {
                "system_template": system_template,
                "instance_template": instance_template,
                "next_step_template": next_step_template,
            },
            "model": {
                "name": model,
                "api_key": f"${api_key_env}",
                "api_base": api_base,
                "per_instance_cost_limit": effective_per_instance_limit,
                "total_cost_limit": effective_total_limit,
                "temperature": 0.0,
                "max_input_tokens": 200000,
                "max_output_tokens": 8192,
            },
            "tools": {
                "bundles": bundles,
                "enable_bash_tool": True,
                "parse_function": {"type": "function_calling"},
            },
        },
    }

    # Include base config reference if default.yaml exists.
    config_path = output_dir / f"sweagent-{arm}.yaml"
    config_path.write_text(yaml.dump(config, default_flow_style=False, sort_keys=False))
    return config_path


def _parse_trajectory(instance_dir: Path, instance_id: str) -> tuple[str, TokenCount, str]:
    """Parse .traj and .pred files; return (patch, TokenCount, exit_status).

    Patch is coerced to "" when model_patch is null (timeout / cost limit).
    cached_input is not surfaced by SWE-agent; left as 0.
    """
    traj_path = instance_dir / f"{instance_id}.traj"
    pred_path = instance_dir / f"{instance_id}.pred"

    if not traj_path.exists():
        raise FileNotFoundError(f"No trajectory file at {traj_path}")

    traj = json.loads(traj_path.read_text())
    info = traj.get("info", {})
    stats = info.get("model_stats", {})
    exit_status = str(info.get("exit_status", "unknown"))

    tokens = TokenCount(
        input=int(stats.get("tokens_sent", 0)),
        cached_input=0,  # not surfaced by SWE-agent
        output=int(stats.get("tokens_received", 0)),
        turns=int(stats.get("api_calls", 0)),
    )

    patch = ""
    if pred_path.exists():
        pred = json.loads(pred_path.read_text())
        patch = pred.get("model_patch") or ""

    return patch, tokens, exit_status


# ---------------------------------------------------------------------------
# Public API
# ---------------------------------------------------------------------------


def run_arm(
    *,
    arm: str,
    task: Task,
    model: str,
    workspace: Path,
    output_dir: Path,
    timeout_seconds: int = 1800,
    blastguard_binary: Path | None = None,
    blastguard_bundle_path: Path | None = None,
) -> ArmResult:
    """Invoke SWE-agent once on *task* for the given *arm*.

    Args:
        arm: "raw" or "blastguard".
        task: Task dataclass with task_id, problem_statement, repo, base_commit.
        model: LiteLLM model name (e.g. "openrouter/minimax/minimax-m2.7").
        workspace: Local repo path. Caller must clone + checkout base_commit first.
        output_dir: Root directory for this arm's outputs. SWE-agent writes
                    <output_dir>/<task_id>/<task_id>.traj and .pred here.
        timeout_seconds: Wall-clock timeout for the SWE-agent subprocess.
        blastguard_binary: Optional path to override the blastguard binary in env.
        blastguard_bundle_path: Optional override for the bundle path (default: BUNDLE_PATH).

    Returns:
        ArmResult with patch, tokens, trajectory_path, exit_status.

    Raises:
        ValueError: Unknown arm value.
        FileNotFoundError: SWE-agent wrote no trajectory (infra failure).
        subprocess.TimeoutExpired: SWE-agent exceeded timeout_seconds.
    """
    if arm not in {"raw", "blastguard"}:
        raise ValueError(f"unknown arm: {arm!r}")

    output_dir.mkdir(parents=True, exist_ok=True)
    workspace.mkdir(parents=True, exist_ok=True)

    # Write the problem statement to a file (flag is --problem_statement.path).
    problem_file = output_dir / "problem.md"
    problem_file.write_text(task.problem_statement)

    # Generate per-invocation config YAML with bundle injection.
    effective_bundle = blastguard_bundle_path or BUNDLE_PATH
    config_path = _build_config_yaml(
        arm=arm,
        model=model,
        output_dir=output_dir,
    )
    # If arm=blastguard was overridden, patch the YAML bundle path.
    if arm == "blastguard" and blastguard_bundle_path is not None:
        _patch_bundle_path(config_path, effective_bundle)

    args = [
        *_sweagent_cmd(),
        "--config", str(config_path),
        "--agent.model.name", model,
        "--env.repo.path", str(workspace),
        "--problem_statement.path", str(problem_file),
        "--output_dir", str(output_dir),
    ]

    env = os.environ.copy()
    if arm == "blastguard":
        env["BLASTGUARD_PROJECT_ROOT"] = str(workspace)
        if blastguard_binary is not None:
            env["BLASTGUARD_BINARY"] = str(blastguard_binary)

    rate_limit_sleep = int(os.environ.get("BENCH_RATE_LIMIT_SLEEP", "60"))
    instance_dir = output_dir / task.task_id
    traj_path = instance_dir / f"{task.task_id}.traj"
    last_proc: subprocess.CompletedProcess[str] | None = None

    for attempt in (1, 2):
        last_proc = subprocess.run(
            args,
            env=env,
            timeout=timeout_seconds,
            capture_output=True,
            text=True,
            check=False,
        )
        if traj_path.exists():
            break
        # Retry once on rate-limit signals; any other failure falls through.
        if attempt == 1 and "rate" in last_proc.stderr.lower():
            time.sleep(rate_limit_sleep)
            continue
        break

    if not traj_path.exists():
        assert last_proc is not None
        raise FileNotFoundError(
            f"sweagent exited {last_proc.returncode} and wrote no trajectory for "
            f"{task.task_id!r} after {attempt} attempt(s). "
            f"stderr:\n{last_proc.stderr[:2000]}"
        )

    patch, tokens, exit_status = _parse_trajectory(instance_dir, task.task_id)
    return ArmResult(
        patch=patch,
        tokens=tokens,
        trajectory_path=traj_path,
        exit_status=exit_status,
    )


def _patch_bundle_path(config_path: Path, bundle_path: Path) -> None:
    """Replace the last bundle entry's path with *bundle_path* in the YAML config."""
    config = yaml.safe_load(config_path.read_text())
    bundles = config.get("agent", {}).get("tools", {}).get("bundles", [])
    if bundles:
        bundles[-1] = {"path": str(bundle_path)}
    config_path.write_text(yaml.dump(config, default_flow_style=False, sort_keys=False))
