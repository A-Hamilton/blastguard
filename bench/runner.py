"""Run one arm (raw or blastguard) against SWE-bench Pro via `sweagent run-batch`.

Emits:
  results/<run_id>/batch/                 — sweagent run-batch output dir
    - preds.jsonl                         — SWE-bench submission format
    - <instance_id>/<hash>.{traj,pred}    — per-task outputs
  results/<run_id>/patches.json           — evaluator input (our format)
  results/<run_id>/telemetry.jsonl        — per-task telemetry
  results/<run_id>/config.json            — arm, seed, model, slice, filter

Usage:
  bench/.venv/bin/python -m bench.runner \\
    --arm blastguard \\
    --model openrouter/minimax/minimax-m2.7 \\
    --limit 10 \\
    --seed 42 \\
    --budget-usd 10.0 \\
    --run-id smoke-blastguard \\
    --blastguard-binary /path/to/blastguard
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from bench.batch_runner import build_batch_config, run_batch
from bench.evaluator import write_patches_json
from bench.prepare_instances import prepare as prepare_instances
from bench.telemetry import TelemetryRecord, write_jsonl

_BUNDLE_PATH = (Path(__file__).parent / "bundles" / "blastguard").resolve()


def _results_dir(run_id: str) -> Path:
    d = Path(__file__).parent / "results" / run_id
    d.mkdir(parents=True, exist_ok=True)
    return d


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--arm", choices=("raw", "blastguard"), required=True)
    p.add_argument(
        "--model",
        default="openrouter/minimax/minimax-m2.7",
        help="LiteLLM-style model name (OpenRouter prefix recommended).",
    )
    p.add_argument("--limit", type=int, default=10, help="Number of tasks (slice :N)")
    p.add_argument("--seed", type=int, default=42, help="shuffle seed (informational)")
    p.add_argument("--budget-usd", type=float, required=True, help="Run-level budget cap (info only)")
    p.add_argument("--run-id", required=True)
    p.add_argument(
        "--per-task-cost-limit",
        type=float,
        default=0.50,
        help="Per-instance cost cap enforced by SWE-agent (USD)",
    )
    p.add_argument(
        "--filter",
        default=".*",
        help="Regex over instance_id to filter tasks (default: all)",
    )
    p.add_argument(
        "--batch-timeout",
        type=int,
        default=7200,
        help="Seconds before the entire batch run is killed",
    )
    p.add_argument("--num-workers", type=int, default=1)
    p.add_argument(
        "--blastguard-binary",
        type=Path,
        default=None,
        help="Path to blastguard release binary (required for arm=blastguard)",
    )
    p.add_argument(
        "--dataset-name",
        default="ScaleAI/SWE-bench_Pro",
        help="HuggingFace dataset name",
    )
    p.add_argument("--split", default="test")
    args = p.parse_args()

    run_dir = _results_dir(args.run_id)
    batch_dir = run_dir / "batch"
    batch_dir.mkdir(parents=True, exist_ok=True)

    (run_dir / "config.json").write_text(
        json.dumps(
            {
                "arm": args.arm,
                "model": args.model,
                "seed": args.seed,
                "budget_usd": args.budget_usd,
                "limit": args.limit,
                "filter": args.filter,
                "per_task_cost_limit": args.per_task_cost_limit,
                "dataset_name": args.dataset_name,
                "split": args.split,
            },
            indent=2,
        )
    )

    # Preprocess HF rows into SimpleBatchInstance-compatible JSONL.
    instances_path = run_dir / "instances.jsonl"
    print(f"[{args.run_id}] preparing instances from {args.dataset_name} (language=python)...")
    prepare_instances(
        dataset_name=args.dataset_name,
        split=args.split,
        language_filter="python",
        limit=None,  # slice happens inside SWE-agent after filter/shuffle
        out_path=instances_path,
    )
    n_instances = sum(1 for _ in instances_path.open())
    print(f"[{args.run_id}] {n_instances} python instances written")

    config_path = build_batch_config(
        arm=args.arm,
        model=args.model,
        instances_path=instances_path,
        instance_filter=args.filter,
        instance_slice=f":{args.limit}" if args.limit else "",
        shuffle=True,
        per_instance_cost_limit=args.per_task_cost_limit,
        output_dir=batch_dir,
    )
    print(f"[{args.run_id}] config: {config_path}")
    print(f"[{args.run_id}] launching sweagent run-batch (arm={args.arm}, limit={args.limit})")

    results = run_batch(
        arm=args.arm,
        model=args.model,
        config_path=config_path,
        output_dir=batch_dir,
        num_workers=args.num_workers,
        timeout_seconds=args.batch_timeout,
        blastguard_binary=args.blastguard_binary,
        blastguard_bundle_path=_BUNDLE_PATH,
    )

    predictions: list[tuple[str, str]] = []
    telemetry: list[TelemetryRecord] = []
    total_cost = 0.0
    for res in results:
        predictions.append((res.task_id, res.patch))
        total_cost += res.cost_usd
        telemetry.append(
            TelemetryRecord(
                task_id=res.task_id,
                arm=args.arm,
                input_tokens=res.tokens.input,
                cached_input_tokens=res.tokens.cached_input,
                output_tokens=res.tokens.output,
                turns=res.tokens.turns,
                wall_seconds=0.0,
                cost_usd=res.cost_usd,
                patch_bytes=len(res.patch.encode("utf-8")),
                error=None if res.exit_status == "submitted" else f"exit_status={res.exit_status}",
            )
        )
        print(
            f"  {res.task_id[:60]} turns={res.tokens.turns} "
            f"cost=${res.cost_usd:.4f} exit={res.exit_status}"
        )

    write_jsonl(telemetry, run_dir / "telemetry.jsonl")
    write_patches_json(
        predictions,
        prefix=f"{args.arm}-{args.model.replace('/', '_')}",
        out_path=run_dir / "patches.json",
    )

    print(f"[{args.run_id}] done — {len(predictions)} tasks, spent ${total_cost:.4f}")
    if args.budget_usd and total_cost > args.budget_usd:
        print(
            f"  WARNING: spend ${total_cost:.4f} exceeded budget ${args.budget_usd:.2f} "
            f"(cap was per-task, not aggregate)"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
