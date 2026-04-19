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

## Current state (2026-04-19)

The harness is feature-complete and live-verified on synthetic tasks:

- `bench/tasks.py` loads ScaleAI/SWE-bench_Pro (Python subset, 266
  instances after filter).
- `bench/prepare_instances.py` transforms HF rows to SWE-agent's
  `SimpleBatchInstance` JSONL.
- `bench/batch_runner.py` invokes `sweagent run-batch` per arm.
- `bench/bundles/blastguard/` is a working SWE-agent tool bundle.
- `bench/scripts/sweagent_with_pricing.py` registers manual pricing for
  LiteLLM-unmapped models before handing off to SWE-agent.
- `bench/stats.py` + `bench/compare.py` do paired McNemar's analysis.
- `bench/evaluator.py` wraps `scaleapi/SWE-bench_Pro-os` subprocess,
  guards against issue #78 silent rate-limit failures.

**End-to-end SWE-bench Pro run is currently blocked upstream.**
SWE-agent's `swerex` deployment does a secondary `docker build
--build-arg BASE_IMAGE=<tag>` that truncates image tags past 128
characters. Most SWE-bench Pro `dockerhub_tag` values exceed this.
See `KNOWN_GAPS.md`.

Pending unblock: either a SWE-agent upstream patch, a local
image-retagging preflight we write, or a pivot to SWE-bench Verified
(which ships `image_name` natively and doesn't hit the truncation path).

## Workflow (Plan 9 — SWE-agent scaffold)

BlastGuard runs as a SWE-agent bundle. SWE-agent handles workspace
cloning, tool dispatch, patch extraction, and timeouts. We orchestrate
per-arm invocations via `bench.runner` and do paired McNemar's analysis
on the outputs.

### Prerequisites

- Docker daemon running (SWE-agent uses Docker per task; the SWE-bench_Pro-os evaluator also requires it)
- `.env` with `OPENROUTER_API_KEY=sk-or-v1-...` (gitignored; export in shell before running)
- `bench/.sweagent-repo/` cloned via `bash bench/scripts/clone_sweagent.sh` (done by Task 1)
- `bench/.evaluator/` cloned via `bash bench/scripts/clone_evaluator.sh`
- `target/release/blastguard` built (`cargo build --release`)
- Bench env up to date: `cd bench && uv sync`

### 1. Harness mock-smoke (no spend, local only)

Run the Task 9 mock-smoke before any real invocation. It replaces the
SWE-agent binary with a local Python stub that emits canned trajectories.
Zero API calls, zero spend.

```bash
cd /home/adam/Documents/blastguard

# Write the mock binary once
cat > /tmp/mock_sweagent.py << 'PYEOF'
import json, os, sys, pathlib
args = sys.argv[1:]
out_dir = None
for i, a in enumerate(args):
    if a == "--output_dir" and i + 1 < len(args):
        out_dir = pathlib.Path(args[i + 1])
if out_dir is None:
    sys.stderr.write("no --output_dir in argv\n"); sys.exit(2)
out_dir.mkdir(parents=True, exist_ok=True)
iid = "unknown"
for i, a in enumerate(args):
    if a == "--instance.instance_id" and i + 1 < len(args):
        iid = args[i + 1]
(out_dir / "trajectory.json").write_text(json.dumps({
    "instance_id": iid,
    "model_stats": {"prompt_tokens": 50000, "completion_tokens": 5000, "n_turns": 12},
    "model_patch": f"diff --git a/readme b/readme\n+ {iid}\n",
}))
sys.exit(0)
PYEOF

export SWEAGENT_BINARY="$(bench/.venv/bin/python -c 'import sys;print(sys.executable)') /tmp/mock_sweagent.py"

# raw arm — 3 tasks, no API calls
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 3 --seed 42 \
  --budget-usd 1.00 --run-id mock-raw \
  --model openrouter/minimax/minimax-m2.7

# blastguard arm — same tasks
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm blastguard --limit 3 --seed 42 \
  --budget-usd 1.00 --run-id mock-bg \
  --model openrouter/minimax/minimax-m2.7

# Verify output shape
bench/.venv/bin/python -c "
import json, pathlib
for run in ('mock-raw', 'mock-bg'):
    p = json.loads((pathlib.Path('bench/results') / run / 'patches.json').read_text())
    print(run, 'tasks:', [e['instance_id'] for e in p])
    assert len(p) == 3
"

# Clean up
rm -rf bench/results/mock-raw bench/results/mock-bg /tmp/mock_sweagent.py
unset SWEAGENT_BINARY
```

Expected: both arms emit 3 predictions with no errors, telemetry
JSONL is well-formed, spend is $0.

### 2. Paired 10-task smoke (~$3-4)

Gate before paying for a pilot. Validates the real SWE-agent + OpenRouter
path end-to-end. The model identifier must carry the LiteLLM
`openrouter/` prefix — bare model names will 404.

```bash
cd /home/adam/Documents/blastguard
export BLASTGUARD_BIN="$(pwd)/target/release/blastguard"
export OPENROUTER_API_KEY="sk-or-v1-..."   # or source from .env

# raw arm
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 10 --seed 42 \
  --budget-usd 5.00 --run-id smoke-raw \
  --model openrouter/minimax/minimax-m2.7

# blastguard arm — same seed, same tasks
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm blastguard --limit 10 --seed 42 \
  --budget-usd 5.00 --run-id smoke-bg \
  --model openrouter/minimax/minimax-m2.7 \
  --blastguard-binary "$BLASTGUARD_BIN"
```

Token and cost data come from `<instance_id>.traj` under each task's
trajectory directory (`info.model_stats.tokens_sent`,
`tokens_received`, `api_calls`, `instance_cost`).

### 3. Grade both arms

```bash
cd /home/adam/Documents/blastguard

bench/.venv/bin/python -c "
from bench.evaluator import run_evaluator
from pathlib import Path
for run_id in ('smoke-raw', 'smoke-bg'):
    rc = run_evaluator(
        evaluator_dir=Path('bench/.evaluator'),
        raw_sample_csv=Path('bench/.evaluator/swe_bench_pro_full.csv'),
        patches_json=Path(f'bench/results/{run_id}/patches.json'),
        output_dir=Path(f'bench/results/{run_id}/eval'),
        num_workers=2,
        timeout_seconds=3600,
    )
    print(f'{run_id} evaluator exit: {rc}')
"
```

### 4. Compare with McNemar's

```bash
cd /home/adam/Documents/blastguard

bench/.venv/bin/python -m bench.compare \
  --raw-output-dir bench/results/smoke-raw/eval \
  --blastguard-output-dir bench/results/smoke-bg/eval
```

Expected output: McNemar's p-value, per-arm resolve rates, delta in pp,
mean tokens per task per arm.

### 5. Pilot → full run gating

- **100-task pilot (~$38 total):** raw arm ~$23, BlastGuard arm ~$14.
  Set `--limit 100` and increase `--budget-usd` accordingly.
  Proceed to the full run only if delta is ≥+1pp trending positive.
- **731-task full run (~$275 total):** raw arm ~$170, BlastGuard arm ~$105.
  Omit `--limit` or set `--limit 731`.

Budget is a hard ceiling — the runner aborts before any call that would
exceed `--budget-usd`. Do not run the full set unless the pilot meets
the gate.

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
