"""Run one arm (raw or blastguard) against the SWE-bench Pro Python subset via SWE-agent.

Emits:
  results/<run_id>/patches.json            — evaluator input
  results/<run_id>/telemetry.jsonl         — per-task telemetry
  results/<run_id>/workspaces/<task_id>/   — cloned repos (reused across retries)
  results/<run_id>/trajectories/<task_id>/ — SWE-agent per-task output dirs
  results/<run_id>/config.json             — arm, seed, model, budget

Usage:
  bench/.venv/bin/python -m bench.runner \\
    --arm blastguard \\
    --model openrouter/minimax/minimax-m2.7 \\
    --limit 10 \\
    --seed 42 \\
    --budget-usd 25.0 \\
    --run-id smoke-blastguard \\
    --blastguard-binary /path/to/blastguard
"""

from __future__ import annotations

import argparse
import json
import random
import subprocess
import time
from pathlib import Path

from bench.budget import Budget, BudgetExceeded
from bench.evaluator import write_patches_json
from bench.sweagent_runner import ArmResult, run_arm
from bench.tasks import load_tasks
from bench.telemetry import TelemetryRecord, append_jsonl

_BUNDLE_PATH = (Path(__file__).parent / "bundles" / "blastguard").resolve()


def _results_dir(run_id: str) -> Path:
    d = Path(__file__).parent / "results" / run_id
    d.mkdir(parents=True, exist_ok=True)
    return d


def _ensure_workspace(workspace: Path, repo: str, base_commit: str) -> str | None:
    """Clone repo if not present and check out base_commit.

    Returns None on success, or an error string if clone/checkout failed.
    """
    if not workspace.exists():
        workspace.mkdir(parents=True, exist_ok=True)
        clone_result = subprocess.run(
            ["git", "clone", f"https://github.com/{repo}.git", str(workspace)],
            capture_output=True,
            text=True,
            check=False,
        )
        if clone_result.returncode != 0:
            return f"git clone failed: {clone_result.stderr[:500]}"

    checkout_result = subprocess.run(
        ["git", "-C", str(workspace), "checkout", "--force", "--detach", base_commit],
        capture_output=True,
        text=True,
        check=False,
    )
    if checkout_result.returncode != 0:
        return f"git checkout {base_commit!r} failed: {checkout_result.stderr[:500]}"

    return None


def main() -> int:  # noqa: PLR0912, PLR0915
    p = argparse.ArgumentParser(
        description="Run one arm against SWE-bench Pro via SWE-agent."
    )
    p.add_argument("--arm", choices=("raw", "blastguard"), required=True)
    p.add_argument(
        "--model",
        default="openrouter/minimax/minimax-m2.7",
        help="LiteLLM model name passed to SWE-agent",
    )
    p.add_argument("--limit", type=int, default=None, help="Cap task count (None = all)")
    p.add_argument("--seed", type=int, default=42, help="Python random seed for task sampling")
    p.add_argument("--budget-usd", type=float, required=True, help="Hard spend ceiling in USD")
    p.add_argument("--run-id", required=True, help="Unique identifier for this run's output dir")
    p.add_argument("--in-price", type=float, default=0.30, help="$/M input tokens")
    p.add_argument("--out-price", type=float, default=1.20, help="$/M output tokens")
    p.add_argument("--cache-price", type=float, default=0.075, help="$/M cached input tokens")
    p.add_argument(
        "--per-task-timeout",
        type=int,
        default=1800,
        help="Seconds before SWE-agent is killed per task",
    )
    p.add_argument(
        "--blastguard-binary",
        type=Path,
        default=None,
        help="Path to blastguard binary (required for blastguard arm)",
    )
    args = p.parse_args()

    random.seed(args.seed)

    run_dir = _results_dir(args.run_id)
    (run_dir / "config.json").write_text(
        json.dumps(
            {
                "arm": args.arm,
                "model": args.model,
                "seed": args.seed,
                "budget_usd": args.budget_usd,
                "limit": args.limit,
                "per_task_timeout": args.per_task_timeout,
            },
            indent=2,
        )
    )

    # Short-circuit: limit=0 emits empty outputs without touching the network.
    if args.limit == 0:
        write_patches_json(
            [],
            prefix=f"{args.arm}-{args.model.replace('/', '_')}",
            out_path=run_dir / "patches.json",
        )
        (run_dir / "telemetry.jsonl").write_text("")
        return 0

    budget = Budget(cap_usd=args.budget_usd)
    tasks = load_tasks(limit=args.limit, python_only=True)
    tasks.sort(key=lambda t: t.task_id)  # identical order across both arms

    telemetry_path = run_dir / "telemetry.jsonl"
    predictions: list[tuple[str, str]] = []
    trajectories_dir = run_dir / "trajectories"
    workspaces_dir = run_dir / "workspaces"

    for task in tasks:
        t0 = time.time()
        task_traj = trajectories_dir / task.task_id
        task_ws = workspaces_dir / task.task_id
        patch = ""
        tokens_input = tokens_cached = tokens_output = tokens_turns = 0
        cost = 0.0
        error: str | None = None
        exit_status = "unknown"

        # Step 1: clone + checkout workspace.
        ws_error = _ensure_workspace(task_ws, task.repo, task.base_commit)
        if ws_error is not None:
            error = ws_error
        else:
            # Step 2: invoke SWE-agent via run_arm.
            try:
                res: ArmResult = run_arm(
                    arm=args.arm,
                    task=task,
                    model=args.model,
                    workspace=task_ws,
                    output_dir=task_traj,
                    timeout_seconds=args.per_task_timeout,
                    blastguard_binary=args.blastguard_binary,
                    blastguard_bundle_path=_BUNDLE_PATH,
                )
                patch = res.patch
                tokens_input = res.tokens.input
                tokens_cached = res.tokens.cached_input
                tokens_output = res.tokens.output
                tokens_turns = res.tokens.turns
                exit_status = res.exit_status

                # Step 3: parse result — flag non-submitted or empty patch.
                if res.exit_status != "submitted" or not res.patch:
                    error = (
                        f"exit_status={res.exit_status}, "
                        f"empty_patch={not res.patch}"
                    )

                # Step 4: record cost against budget.
                try:
                    cost = budget.record(
                        input_tokens=tokens_input,
                        cached_input_tokens=tokens_cached,
                        output_tokens=tokens_output,
                        in_price_per_m=args.in_price,
                        cache_read_per_m=args.cache_price,
                        out_price_per_m=args.out_price,
                    )
                except BudgetExceeded as e:
                    print(f"[{task.task_id}] BUDGET STOP: {e}")  # noqa: T201
                    predictions.append((task.task_id, patch))
                    append_jsonl(
                        TelemetryRecord(
                            task_id=task.task_id,
                            arm=args.arm,
                            input_tokens=tokens_input,
                            cached_input_tokens=tokens_cached,
                            output_tokens=tokens_output,
                            turns=tokens_turns,
                            wall_seconds=time.time() - t0,
                            cost_usd=0.0,
                            patch_bytes=len(patch.encode()),
                            error=f"budget_exceeded: {e}",
                        ),
                        telemetry_path,
                    )
                    break

            except Exception as e:  # noqa: BLE001 — intentional log + continue
                error = f"{type(e).__name__}: {e}"
                print(f"[{task.task_id}] ERROR: {error}")  # noqa: T201

        predictions.append((task.task_id, patch))
        append_jsonl(
            TelemetryRecord(
                task_id=task.task_id,
                arm=args.arm,
                input_tokens=tokens_input,
                cached_input_tokens=tokens_cached,
                output_tokens=tokens_output,
                turns=tokens_turns,
                wall_seconds=time.time() - t0,
                cost_usd=cost,
                patch_bytes=len(patch.encode()),
                error=error,
            ),
            telemetry_path,
        )
        print(  # noqa: T201
            f"[{task.task_id}] exit={exit_status} turns={tokens_turns} "
            f"tokens={tokens_input + tokens_output} "
            f"cost=${cost:.4f} spent=${budget.spent_usd:.4f}"
        )

    write_patches_json(
        predictions,
        prefix=f"{args.arm}-{args.model.replace('/', '_')}",
        out_path=run_dir / "patches.json",
    )
    print(  # noqa: T201
        f"done: wrote {len(predictions)} predictions; spent ${budget.spent_usd:.4f}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
