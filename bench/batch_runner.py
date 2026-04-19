"""Orchestrate `sweagent run-batch` — one invocation per arm.

Why `run-batch` instead of per-task `sweagent run`:

- `run-batch` uses SWE-bench Pro's built-in Docker images per task
  (`dockerhub_tag` column). Per-task `run` defaults to plain
  python:3.11 which lacks task-specific dependencies.
- `run-batch` handles repo checkout + base_commit internally; no
  external `git clone` orchestration needed.
- Parallelism via `--num_workers`.
- Trajectory/pred output structure is standard and easy to parse.

`sweagent run-batch` writes to `<output_dir>/<instance_id>/<hash>.{traj,pred}`
and aggregates all `.pred` files into `preds.jsonl` in SWE-bench submission
format. We parse both: telemetry per task from `.traj`, final patches from
`preds.jsonl`.
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import yaml

from bench.sweagent_runner import (
    DEFAULT_PER_INSTANCE_CALL_LIMIT,
    MODEL_PRICING_USD_PER_TOKEN,
    _model_is_mapped_in_litellm,
)
from bench.token_count import TokenCount

REPO_ROOT = Path(__file__).resolve().parent.parent
BUNDLE_PATH = REPO_ROOT / "bench" / "bundles" / "blastguard"
_SWEAGENT_REPO = Path(__file__).resolve().parent / ".sweagent-repo"


@dataclass(frozen=True, slots=True)
class BatchTaskResult:
    """Per-task result within a batch run."""

    task_id: str
    patch: str
    tokens: TokenCount
    exit_status: str
    cost_usd: float


def _sweagent_cmd() -> list[str]:
    """Resolve the sweagent binary, honoring SWEAGENT_BINARY for tests."""
    override = os.environ.get("SWEAGENT_BINARY")
    if override:
        return shlex.split(override)
    return ["sweagent", "run-batch"]


def build_batch_config(
    *,
    arm: str,
    model: str,
    instances_path: Path,
    instance_filter: str = ".*",
    instance_slice: str = "",
    shuffle: bool = True,
    per_instance_cost_limit: float = 0.50,
    output_dir: Path,
    api_key_env: str = "OPENROUTER_API_KEY",
    api_base: str = "https://openrouter.ai/api/v1",
) -> Path:
    """Write a `run-batch` config YAML and return its path.

    The YAML extends SWE-agent's default.yaml — inherits the system/instance
    templates, the tool registry (which provides `submit`), and history
    processors. On arm=blastguard we additionally register the BlastGuard
    bundle and append BLASTGUARD_BIAS to the system template.
    """
    default_config_path = _SWEAGENT_REPO / "config" / "default.yaml"
    config: dict[str, Any] = (
        yaml.safe_load(default_config_path.read_text()) or {} if default_config_path.exists() else {}
    )

    agent_cfg = config.setdefault("agent", {})
    tools_cfg = agent_cfg.setdefault("tools", {})
    bundles_cfg: list[dict[str, Any]] = tools_cfg.setdefault("bundles", [])

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

    if arm == "blastguard":
        from bench.prompts import BLASTGUARD_BIAS  # noqa: PLC0415

        templates = agent_cfg.setdefault("templates", {})
        templates["system_template"] = templates.get("system_template", "") + "\n\n" + BLASTGUARD_BIAS

    is_free_tier = model.endswith(":free")
    is_litellm_mapped = _model_is_mapped_in_litellm(model)
    manual_pricing = MODEL_PRICING_USD_PER_TOKEN.get(model)
    can_enforce_cost_cap = is_litellm_mapped or manual_pricing is not None
    effective_per_instance_limit = (
        per_instance_cost_limit if can_enforce_cost_cap and not is_free_tier else 0.0
    )

    agent_cfg["model"] = {
        "name": model,
        "api_key": f"${api_key_env}",
        "api_base": api_base,
        "per_instance_cost_limit": effective_per_instance_limit,
        "total_cost_limit": 0.0,
        "per_instance_call_limit": DEFAULT_PER_INSTANCE_CALL_LIMIT,
        "temperature": 0.0,
        "max_input_tokens": 200000,
        "max_output_tokens": 8192,
    }

    # Instances block — uses the file loader. We pre-process HF rows via
    # bench/prepare_instances.py because SWE-bench Pro ships with
    # `dockerhub_tag` but SWE-agent's InstancesFromHuggingFace expects
    # `image_name` in the schema.
    config["instances"] = {
        "type": "file",
        "path": str(instances_path),
        "filter": instance_filter,
        "slice": instance_slice,
        "shuffle": shuffle,
    }

    output_dir.mkdir(parents=True, exist_ok=True)
    config_path = output_dir / f"sweagent-batch-{arm}.yaml"
    config_path.write_text(yaml.dump(config, default_flow_style=False, sort_keys=False))
    return config_path


def run_batch(
    *,
    arm: str,
    model: str,
    config_path: Path,
    output_dir: Path,
    num_workers: int = 1,
    timeout_seconds: int = 7200,
    blastguard_binary: Path | None = None,
    blastguard_bundle_path: Path | None = None,
) -> list[BatchTaskResult]:
    """Invoke `sweagent run-batch` and parse the per-task outputs.

    Returns one `BatchTaskResult` per completed task (failed tasks included
    with exit_status set and empty patch so compare.py flags them as
    infra_failure).
    """
    output_dir.mkdir(parents=True, exist_ok=True)

    args = [
        *_sweagent_cmd(),
        "--config", str(config_path),
        "--agent.model.name", model,
        "--output_dir", str(output_dir),
        "--num_workers", str(num_workers),
    ]

    env = os.environ.copy()
    if arm == "blastguard":
        if blastguard_binary is not None:
            env["BLASTGUARD_BINARY"] = str(blastguard_binary)
        if blastguard_bundle_path is not None:
            env["BLASTGUARD_PROJECT_ROOT"] = str(blastguard_bundle_path)

    subprocess.run(
        args,
        env=env,
        timeout=timeout_seconds,
        check=False,
    )
    return parse_batch_outputs(output_dir)


def parse_batch_outputs(output_dir: Path) -> list[BatchTaskResult]:
    """Walk the batch output directory, building one result per `.traj`.

    Layout (SWE-agent run-batch):
        <output_dir>/<instance_id>/<hash>.traj
        <output_dir>/<instance_id>/<hash>.pred
        <output_dir>/preds.jsonl          (aggregate submissions)
    """
    results: list[BatchTaskResult] = []
    for traj_path in sorted(output_dir.glob("*/*.traj")):
        instance_dir = traj_path.parent
        task_id = instance_dir.name
        pred_path = traj_path.with_suffix(".pred")

        try:
            traj = json.loads(traj_path.read_text())
        except json.JSONDecodeError:
            results.append(
                BatchTaskResult(
                    task_id=task_id,
                    patch="",
                    tokens=TokenCount(0, 0, 0, 0),
                    exit_status="json_decode_error",
                    cost_usd=0.0,
                )
            )
            continue

        info = traj.get("info", {})
        stats = info.get("model_stats", {})
        exit_status = str(info.get("exit_status", "unknown"))
        tokens = TokenCount(
            input=int(stats.get("tokens_sent", 0)),
            cached_input=0,
            output=int(stats.get("tokens_received", 0)),
            turns=int(stats.get("api_calls", 0)),
        )
        cost = float(stats.get("instance_cost", 0.0) or 0.0)

        patch = ""
        if pred_path.exists():
            try:
                pred = json.loads(pred_path.read_text())
                patch = pred.get("model_patch") or ""
            except json.JSONDecodeError:
                exit_status = "pred_decode_error"

        results.append(
            BatchTaskResult(
                task_id=task_id,
                patch=patch,
                tokens=tokens,
                exit_status=exit_status,
                cost_usd=cost,
            )
        )
    return results
