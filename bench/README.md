# BlastGuard Benchmark Harness

> **⚠️ NOT FAITHFUL YET.** This harness is a Plan 7 skeleton. Empirical
> contact with the real dataset on 2026-04-18 exposed several gaps that
> must be closed before it produces numbers comparable to the official
> [SWE-bench Pro leaderboard](https://labs.scale.com/leaderboard/swe_bench_pro_public).
> See [`KNOWN_GAPS.md`](KNOWN_GAPS.md) in this directory before running
> anything.

Planned end-to-end SWE-bench Pro harness per SPEC §15. The skeleton
loads tasks, runs an agent loop, and emits per-task JSONL. What's
missing: Docker-based grading via `jefzda/sweap-images`, multi-language
test runners, and the full dataset schema mapping.

## Setup

```bash
# Build the BlastGuard binary (required).
cd /home/adam/Documents/blastguard
cargo build --release

# Install the harness deps.
cd bench
uv sync

# Configure credentials.
export ANTHROPIC_API_KEY=sk-ant-...
export OPENROUTER_API_KEY=sk-or-...   # for GLM-5.1
```

## Workflow (Plan 8)

Prerequisites:
- Docker daemon running (the evaluator pulls per-repo images)
- `.env` with `OPENROUTER_API_KEY=sk-or-v1-...`
- `bench/.evaluator/` cloned via `bash bench/scripts/clone_evaluator.sh`

### 1. Paired smoke (10 tasks, ~$3-4)

Gate before paying for a pilot. Validates harness end-to-end.

```bash
cd /home/adam/Documents/blastguard

# raw arm
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 10 --seed 42 \
  --budget-usd 5.00 --run-id smoke-raw \
  --model minimax/minimax-m2.7

# blastguard arm — same seed, same tasks
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm blastguard --limit 10 --seed 42 \
  --budget-usd 5.00 --run-id smoke-bg \
  --model minimax/minimax-m2.7
```

### 2. Paired pilot (100 tasks, ~$38 total)

Raw arm ~$23 (more tokens, more turns). BlastGuard arm ~$14 (graph retrieval cuts per-task tokens ~40%).

```bash
cd /home/adam/Documents/blastguard

# raw arm — budget at expected ceiling
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 100 --seed 42 \
  --budget-usd 30.00 --run-id pilot-raw \
  --model minimax/minimax-m2.7

# blastguard arm — same seed, same tasks
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm blastguard --limit 100 --seed 42 \
  --budget-usd 20.00 --run-id pilot-bg \
  --model minimax/minimax-m2.7
```

### 3. Grade both arms

```bash
cd /home/adam/Documents/blastguard

bench/.venv/bin/python -c "
from bench.evaluator import run_evaluator
from pathlib import Path
for run_id in ('pilot-raw', 'pilot-bg'):
    rc = run_evaluator(
        evaluator_dir=Path('bench/.evaluator'),
        raw_sample_csv=Path('bench/.evaluator/swe_bench_pro_full.csv'),
        patches_json=Path(f'bench/results/{run_id}/patches.json'),
        output_dir=Path(f'bench/results/{run_id}/eval'),
        num_workers=4,
        timeout_seconds=3600,
    )
    print(f'{run_id} evaluator exit: {rc}')
"
```

### 4. Compare with McNemar's

```bash
cd /home/adam/Documents/blastguard

bench/.venv/bin/python -m bench.compare \
  --raw-output-dir bench/results/pilot-raw/eval \
  --blastguard-output-dir bench/results/pilot-bg/eval
```

Expected output: McNemar's p-value, per-arm scores, delta in pp, mean
tokens per task per arm.

### 5. Full run (731 tasks, ~$275 total) — gated on pilot showing ≥+1pp delta

Swap `--limit 100` for `--limit 731` (or omit `--limit` entirely).
Budgets: raw ~$180, BlastGuard ~$110. Do not run this step unless the
pilot shows ≥+1pp delta at p < 0.05.

## Methodology

- **Scaffold:** minimal single-turn-with-tool-use loop. Agent receives
  the task's `problem_statement`, uses native `bash` + `str_replace_editor`
  (and, when enabled, BlastGuard's `search` / `apply_change` / `run_tests`).
- **Turn cap:** 50 per task.
- **Grading:** SPEC §15.4 BenchJack defense — pytest runs with `-p
  no:cacheprovider -o addopts=`, any modification to `conftest.py` /
  `pytest.ini` / `tox.ini` / `.github/workflows/**` is classified as
  tampering and counts as unresolved.
- **Isolation:** each task runs in a throwaway tempdir clone at the
  task's `base_commit`. Workspaces are NOT reused between tasks.

## Honesty contract (per SPEC §15.3)

Publish measured results with confidence intervals, not projected
numbers. If BlastGuard shows 0 or negative lift, we publish that. The
README at the repo root cites the latest run's numbers and links to
the raw JSONL in `bench/results/`.
