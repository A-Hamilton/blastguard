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

## Smoke run (3 tasks, ~$0.50 of Claude API)

```bash
cd bench
uv run python runner.py --tasks 3 --model claude-opus-4-7 --no-blastguard \
    --output results/baseline-smoke.jsonl
uv run python runner.py --tasks 3 --model claude-opus-4-7 --with-blastguard \
    --output results/blastguard-smoke.jsonl
uv run python compare.py results/baseline-smoke.jsonl results/blastguard-smoke.jsonl
```

Expected output: a printed comparison block with resolution-rate delta,
token delta, per-repo breakdown.

## Full run (731 tasks × 2 conditions × 2 models)

This is a paid operation. Estimated cost ≈ $300-500 depending on the
model's per-task turn count. Run in the background overnight and
commit the results to `bench/results/`:

```bash
cd bench

# Baseline — no BlastGuard, Claude Opus 4.7.
uv run python runner.py --tasks 731 --model claude-opus-4-7 --no-blastguard \
    --output results/baseline-opus-4-7.jsonl

# BlastGuard — Claude Opus 4.7.
uv run python runner.py --tasks 731 --model claude-opus-4-7 --with-blastguard \
    --output results/blastguard-opus-4-7.jsonl

# Baseline — GLM-5.1.
uv run python runner.py --tasks 731 --model glm-5.1 --provider openai --no-blastguard \
    --output results/baseline-glm-5-1.jsonl

# BlastGuard — GLM-5.1.
uv run python runner.py --tasks 731 --model glm-5.1 --provider openai --with-blastguard \
    --output results/blastguard-glm-5-1.jsonl

# Compare per model.
uv run python compare.py results/baseline-opus-4-7.jsonl results/blastguard-opus-4-7.jsonl
uv run python compare.py results/baseline-glm-5-1.jsonl results/blastguard-glm-5-1.jsonl
```

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
