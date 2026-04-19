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


def _model_is_mapped_in_litellm(model: str) -> bool:
    """Return True if LiteLLM knows how to price this model.

    LiteLLM's `model_cost` table covers only a subset of providers/models.
    Unmapped models trigger SWE-agent's ModelConfigurationError unless we
    disable its cost tracking. We do our own cost tracking in bench.budget,
    so bypassing SWE-agent's is safe.
    """
    try:
        import litellm  # noqa: PLC0415

        litellm.get_model_info(model)
        return True
    except Exception:  # noqa: BLE001 — any lookup failure = unmapped
        return False


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
    # LiteLLM's price table covers only a subset of models. SWE-agent raises
    # ModelConfigurationError for any model it can't price unless both cost
    # limits are 0. We detect unmapped models at config-gen time and bypass
    # SWE-agent's cost tracking — our own bench.budget tracks spend via the
    # trajectory's token counts after each task. Free-tier (`:free` suffix)
    # models also bypass since they cost $0 anyway.
    is_free_tier = model.endswith(":free")
    is_litellm_mapped = _model_is_mapped_in_litellm(model)
    bypass_cost_tracking = is_free_tier or not is_litellm_mapped
    effective_per_instance_limit = 0.0 if bypass_cost_tracking else per_instance_cost_limit
    effective_total_limit = 0.0

    # Start from SWE-agent's default.yaml so we inherit its carefully-crafted
    # system_template, instance_template, tool-registry bundles (including
    # `submit`, `edit_anthropic`, and the filemap state script), and history
    # processors. Overriding these from scratch (our earlier attempt) left the
    # agent without the `submit` tool — it kept invoking `submit` via bash
    # until hitting the turn limit.
    default_config_path = _SWEAGENT_REPO / "config" / "default.yaml"
    if default_config_path.exists():
        config: dict[str, Any] = yaml.safe_load(default_config_path.read_text()) or {}
    else:
        config = {}

    agent_cfg = config.setdefault("agent", {})
    tools_cfg = agent_cfg.setdefault("tools", {})
    bundles_cfg: list[dict[str, Any]] = tools_cfg.setdefault("bundles", [])

    # Ensure bundle paths are absolute (defaults are relative to the repo root).
    resolved: list[dict[str, Any]] = []
    for b in bundles_cfg:
        p = Path(b["path"])
        if not p.is_absolute():
            p = _SWEAGENT_REPO / p
        resolved.append({"path": str(p)})
    bundles_cfg = resolved

    if arm == "blastguard":
        bundles_cfg.append({"path": str(BUNDLE_PATH)})
    tools_cfg["bundles"] = bundles_cfg

    # Append BlastGuard steering to the default system_template; don't replace
    # it — the default carries rules the registry bundle relies on.
    if arm == "blastguard":
        from bench.prompts import BLASTGUARD_BIAS  # noqa: PLC0415

        templates = agent_cfg.setdefault("templates", {})
        base_sys = templates.get("system_template", "")
        templates["system_template"] = base_sys + "\n\n" + BLASTGUARD_BIAS

    # Model config overrides (OpenRouter routing + cost bypass).
    agent_cfg["model"] = {
        "name": model,
        "api_key": f"${api_key_env}",
        "api_base": api_base,
        "per_instance_cost_limit": effective_per_instance_limit,
        "total_cost_limit": effective_total_limit,
        "temperature": 0.0,
        "max_input_tokens": 200000,
        "max_output_tokens": 8192,
    }

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
        if _find_trajectory_file(output_dir) is not None:
            break
        # Retry once on rate-limit signals; any other failure falls through.
        if attempt == 1 and "rate" in last_proc.stderr.lower():
            time.sleep(rate_limit_sleep)
            continue
        break

    traj_path = _find_trajectory_file(output_dir)
    if traj_path is None:
        assert last_proc is not None
        raise FileNotFoundError(
            f"sweagent exited {last_proc.returncode} and wrote no trajectory for "
            f"{task.task_id!r} after {attempt} attempt(s). "
            f"stderr:\n{last_proc.stderr[:2000]}"
        )

    # SWE-agent hashes the task_id into a short subdirectory name (e.g.
    # "verify-synthetic-add-bug" → "2c862e/") — so instance_dir and the
    # filename stem come from the trajectory file, not task.task_id.
    instance_dir = traj_path.parent
    instance_stem = traj_path.stem
    patch, tokens, exit_status = _parse_trajectory(instance_dir, instance_stem)
    return ArmResult(
        patch=patch,
        tokens=tokens,
        trajectory_path=traj_path,
        exit_status=exit_status,
    )


def _find_trajectory_file(output_dir: Path) -> Path | None:
    """Return the first `*.traj` file under *output_dir* (or None).

    SWE-agent derives the per-instance subdirectory name via a short hash of
    the instance_id, which we can't reproduce easily. Globbing is reliable:
    each run emits exactly one trajectory file in exactly one subdirectory.
    """
    matches = list(output_dir.glob("*/*.traj"))
    return matches[0] if matches else None


def _patch_bundle_path(config_path: Path, bundle_path: Path) -> None:
    """Replace the last bundle entry's path with *bundle_path* in the YAML config."""
    config = yaml.safe_load(config_path.read_text())
    bundles = config.get("agent", {}).get("tools", {}).get("bundles", [])
    if bundles:
        bundles[-1] = {"path": str(bundle_path)}
    config_path.write_text(yaml.dump(config, default_flow_style=False, sort_keys=False))
