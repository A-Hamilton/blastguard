# Benchmarking BlastGuard

This document explains the `bench/` harness architecture, how to run it,
and what's currently gating an end-to-end SWE-bench Pro result.

## What the harness does

A **paired** measurement of BlastGuard's lift on SWE-bench-style tasks.
Same model, same tasks, same seed; the only variable is whether the
agent has BlastGuard available as an MCP tool.

- **Arm A (raw):** SWE-agent with its default tool registry.
- **Arm B (BlastGuard):** identical scaffold plus a tool bundle exposing
  `blastguard_search`, `blastguard_apply_change`, and
  `blastguard_run_tests` over an MCP bridge.

Per-task outcomes get paired by `instance_id` and fed through
[McNemar's test](https://en.wikipedia.org/wiki/McNemar%27s_test)
(`bench/stats.py`) to distinguish real lift from run-to-run noise.

## Architecture

```
  HF Dataset              prepare_instances.py          batch_runner.py
  (ScaleAI/SWE-bench_Pro)  →  instances.jsonl  →  build_batch_config()
                                                         ↓
                                                   sweagent run-batch
                                                         ↓
                                                   preds.jsonl
                                                         ↓
                                                   evaluator.py (SWE-bench_Pro-os)
                                                         ↓
                                                   compare.py (McNemar's)
```

Budget, telemetry, and BenchJack tamper defense live in:
- `bench/budget.py` (post-hoc cost tracking)
- `bench/telemetry.py` (per-task JSONL writer)
- `bench/evaluator.py::detect_tampering` (conftest.py / workflow-file
  rejection; the tamper vectors are documented in the grader source)

## How to run (once upstream is unblocked)

Prerequisites: Docker, OpenRouter API key, HuggingFace token,
`bench/.sweagent-repo/` cloned via `bench/scripts/clone_sweagent.sh`,
`target/release/blastguard` built via `cargo build --release`,
`bench/.evaluator/` cloned via `bench/scripts/clone_evaluator.sh`.

```bash
cd /path/to/blastguard
export OPENROUTER_API_KEY="sk-or-v1-..."
export HF_TOKEN="hf_..."
export SWE_AGENT_CONFIG_DIR="$(pwd)/bench/.sweagent-repo/config"
export SWE_AGENT_TOOLS_DIR="$(pwd)/bench/.sweagent-repo/tools"
export SWE_AGENT_TRAJECTORY_DIR="$(pwd)/bench/.sweagent-repo/trajectories"
export SWEAGENT_BINARY="$(pwd)/bench/.venv/bin/python $(pwd)/bench/scripts/sweagent_with_pricing.py run-batch"

# 10-task paired smoke, ~$5
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 10 --seed 42 --budget-usd 5 \
  --run-id smoke-raw --model openrouter/minimax/minimax-m2.7 \
  --per-task-cost-limit 0.50 --batch-timeout 3600

HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm blastguard --limit 10 --seed 42 --budget-usd 5 \
  --run-id smoke-bg --model openrouter/minimax/minimax-m2.7 \
  --per-task-cost-limit 0.50 --batch-timeout 3600 \
  --blastguard-binary "$(pwd)/target/release/blastguard"

# Grade
bench/.venv/bin/python -c "
from bench.evaluator import run_evaluator
from pathlib import Path
for r in ('smoke-raw', 'smoke-bg'):
    run_evaluator(
        evaluator_dir=Path('bench/.evaluator'),
        raw_sample_csv=Path('bench/.evaluator/swe_bench_pro_full.csv'),
        patches_json=Path(f'bench/results/{r}/patches.json'),
        output_dir=Path(f'bench/results/{r}/eval'),
    )"

# Compare
bench/.venv/bin/python -m bench.compare \
  --raw-output-dir bench/results/smoke-raw/eval \
  --blastguard-output-dir bench/results/smoke-bg/eval
```

## Why a run hasn't been published

See `bench/KNOWN_GAPS.md` Gap 5. The short version: SWE-agent's Docker
deployment truncates SWE-bench Pro image tags past 128 chars and every
task fails at environment setup. We've invested in three workarounds
(manual pricing registration, per-instance call caps, timeout-trajectory
rescue) but the truncation itself needs an upstream fix, a preflight
re-tagging pass, or a pivot to SWE-bench Verified.

## Guardrails that are live

- **Turn cap** (`bench/sweagent_runner.py::DEFAULT_PER_INSTANCE_CALL_LIMIT`):
  40 API calls per task.
- **Cost cap** (config `per_instance_cost_limit`): defaults to $0.50 per
  task when the model is LiteLLM-priced or carries manual pricing.
- **Run-level budget** (`bench/budget.py`): post-hoc check, raises
  `BudgetExceeded` if a record() call would cross the cap.
- **BenchJack defense** (`bench/evaluator.py::detect_tampering`):
  flags edits to `conftest.py`, `pytest.ini`, `pyproject.toml`,
  `setup.cfg`, `tox.ini`, or `.github/workflows/**`.
- **Infra-failure filter** (`bench/compare.py::pair_results`):
  excludes tasks where either arm hit an evaluator error, empty patch,
  or SWE-agent non-`submitted` exit from McNemar's counts.
