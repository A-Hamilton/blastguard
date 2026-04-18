"""Run one arm (raw or blastguard) against the SWE-bench Pro Python subset.

Emits:
  results/<run_id>/patches.json            — evaluator input
  results/<run_id>/telemetry.jsonl         — per-task telemetry
  results/<run_id>/config.json             — arm, seed, model, budget

Usage:
  uv run python -m bench.runner \\
    --arm blastguard \\
    --model minimax/minimax-m2.7 \\
    --limit 10 \\
    --seed 42 \\
    --budget-usd 25.0 \\
    --run-id smoke-blastguard
"""

from __future__ import annotations

import argparse
import json
import random
import time
from pathlib import Path

from bench.budget import Budget, BudgetExceeded
from bench.evaluator import write_patches_json
from bench.prompts import build_system_prompt
from bench.tasks import load_tasks
from bench.telemetry import TelemetryRecord, append_jsonl


def _results_dir(run_id: str) -> Path:
    d = Path(__file__).parent / "results" / run_id
    d.mkdir(parents=True, exist_ok=True)
    return d


def main() -> int:  # noqa: PLR0912, PLR0915
    p = argparse.ArgumentParser()
    p.add_argument("--arm", choices=("raw", "blastguard"), required=True)
    p.add_argument("--model", default="minimax/minimax-m2.7")
    p.add_argument("--limit", type=int, default=None, help="Cap task count")
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--budget-usd", type=float, required=True)
    p.add_argument("--run-id", required=True)
    p.add_argument("--in-price", type=float, default=0.30, help="$/M input tokens")
    p.add_argument("--out-price", type=float, default=1.20, help="$/M output tokens")
    p.add_argument("--cache-price", type=float, default=0.075, help="$/M cached input")
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
            },
            indent=2,
        )
    )

    # Short-circuit: if limit is 0, emit empty outputs and exit cleanly.
    # This avoids constructing the API client (which would require env vars).
    if args.limit == 0:
        write_patches_json(
            [],
            prefix=f"{args.arm}-{args.model.replace('/', '_')}",
            out_path=run_dir / "patches.json",
        )
        (run_dir / "telemetry.jsonl").write_text("")
        return 0

    # Deferred imports so --limit 0 never touches network clients.
    from bench.agent_loop import TokenCount, run_openai_compatible  # noqa: PLC0415
    from bench.mcp_client import BlastGuardClient  # noqa: PLC0415

    budget = Budget(cap_usd=args.budget_usd)
    tasks = load_tasks(limit=args.limit, python_only=True)
    # Sort by task_id so both arms iterate in the same order.
    tasks.sort(key=lambda t: t.task_id)

    telemetry_path = run_dir / "telemetry.jsonl"
    predictions: list[tuple[str, str]] = []
    mcp_client: BlastGuardClient | None = BlastGuardClient() if args.arm == "blastguard" else None
    if mcp_client is not None:
        mcp_client.start()

    try:
        for task in tasks:
            t0 = time.time()
            patch = ""
            cost = 0.0
            error: str | None = None
            tokens = TokenCount(input=0, cached_input=0, output=0, turns=0)
            try:
                patch, tokens = run_openai_compatible(
                    model=args.model,
                    system_prompt=build_system_prompt(arm=args.arm),
                    problem_statement=task.problem_statement,
                    mcp_client=mcp_client,
                    seed=args.seed,
                )
                cost = budget.record(
                    input_tokens=tokens.input,
                    cached_input_tokens=tokens.cached_input,
                    output_tokens=tokens.output,
                    in_price_per_m=args.in_price,
                    cache_read_per_m=args.cache_price,
                    out_price_per_m=args.out_price,
                )
                error = None
            except BudgetExceeded as e:
                print(f"[{task.task_id}] BUDGET STOP: {e}")  # noqa: T201
                break
            except Exception as e:  # noqa: BLE001 — intentional, log + continue
                patch = ""
                cost = 0.0
                error = f"{type(e).__name__}: {e}"

            predictions.append((task.task_id, patch))
            append_jsonl(
                TelemetryRecord(
                    task_id=task.task_id,
                    arm=args.arm,
                    input_tokens=tokens.input,
                    cached_input_tokens=tokens.cached_input,
                    output_tokens=tokens.output,
                    turns=tokens.turns,
                    wall_seconds=time.time() - t0,
                    cost_usd=cost,
                    patch_bytes=len(patch.encode("utf-8")),
                    error=error,
                ),
                telemetry_path,
            )
            print(f"[{task.task_id}] cost=${cost:.4f} spent=${budget.spent_usd:.4f}")  # noqa: T201
    finally:
        if mcp_client is not None:
            mcp_client.stop()

    write_patches_json(
        predictions,
        prefix=f"{args.arm}-{args.model.replace('/', '_')}",
        out_path=run_dir / "patches.json",
    )
    print(f"done: wrote {len(predictions)} predictions; spent ${budget.spent_usd:.4f}")  # noqa: T201
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
