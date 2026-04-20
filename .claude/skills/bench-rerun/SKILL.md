---
name: bench-rerun
description: Run a Gemma-4 microbench task (or the full suite) against BlastGuard with the strict setup ritual. Use when you want to measure the impact of a recent change on agent efficiency.
disable-model-invocation: true
---

# Bench Rerun (Gemma-4 26B, local, zero API cost)

User-only: Claude will not invoke this automatically. Type `/bench-rerun` or `/bench-rerun <task_id>` to run.

## Setup ritual (every run)

All steps matter — cutting any one of them produced bad data last time:

1. **Ensure llama-swap is up AND Gemma is actually responsive** (not just the `/models` endpoint — that stays up even when Gemma is unloaded):
   ```bash
   curl -m 30 -H "Content-Type: application/json" -H "Authorization: Bearer sk-local" \
     -d '{"model":"gemma-4","messages":[{"role":"user","content":"hi"}],"max_tokens":5}' \
     http://127.0.0.1:8080/v1/chat/completions
   ```
   Expect a JSON response within ~5s of a cold start. A 30s timeout with zero bytes received means Gemma is hung / OOM-crashed and needs a llama-swap restart (`systemctl --user restart llama-swap`).

2. **Clear BlastGuard's index cache** so the rollout measures a true cold index, not a cached-warm one.
   ```bash
   test -d .blastguard && rm -r .blastguard
   ```

3. **Rebuild the release binary** (the bench calls it over MCP stdio).
   ```bash
   cargo build --release 2>&1 | tail -3
   ```
   Expect `Finished \`release\` profile`.

4. **Confirm llama-swap's TTL is ≥ 7200s.** Default 300s causes mid-run connection refused errors on multi-task suites. Check `~/.config/llama-swap/config.yaml` — both `globalTTL` and the Gemma model's `ttl:` must be 7200.

5. **Context window vs VRAM — `-c 16384` is the sweet spot for 17GB cards.** `-c 32768` pushed a Q4_K_M 26B A4B model to the edge of a 17GB GPU in round 9, causing intermittent silent OOM hangs (llama-server evicted mid-rollout, llama-swap's reload-on-demand stalled, Python hung on the HTTP call). BlastGuard rollouts don't need 32K — round-8 peak per-turn input was ~9K tokens, so 16K leaves ~2× headroom. If the card is ≥24GB, `-c 32768` is fine.

6. **If Gemma hangs mid-run**: check VRAM with `rocm-smi --showmeminfo vram`. If `VRAM Used` is near `VRAM Total`, you hit OOM — reduce `-c`, reload llama-swap, retry. If `VRAM Used` is low but requests still hang, llama-swap failed to reload the model; a full `systemctl --user restart llama-swap` fixes it.

## Run

### Single task with quality grading (fast feedback)

```bash
bench/.venv/bin/python -m bench.microbench \
  --api-base http://127.0.0.1:8080/v1 \
  --api-key-env DUMMY_KEY \
  --model gemma-4 --model-id-override gemma-4 \
  --tasks <task_id> \
  --seeds 3 \
  --run-judge --judge-n 3 \
  --output bench/runs/$(date +%Y%m%d-%H%M%S)-<task_id>.jsonl
```

Typical task IDs: see `bench/tasks_registry.py`. At time of writing:
`explore-cold-index`, `callers-apply-edit`, `chain-search-to-graph`,
`cascade-signature-change`, `outline-tree-sitter-rust`,
`trace-cache-persistence`, `find-tamper-patterns`,
`impact-of-removing-libraries`, `compare-parse-modules`,
`tests-for-apply-change`.

### Full suite (slow — 30–60 min)

Drop `--tasks` to run every task. Expect 10 tasks × 2 arms × seeds
rollouts, plus same-(task, seed) judge calls if `--run-judge` is
set.

## Analyse

The microbench harness now prints a priority-ordered summary at
the end of each run automatically:

- **Priority 1a — deterministic substring grader** (always runs).
  Per (task, arm) correctness rate, plus a `VERDICT: COMMIT OK`
  or `DO NOT COMMIT` line. BG must stay within 2pp of raw per
  task. See `bench/microbench_grader.py`.
- **Priority 1b — LLM-as-judge pairwise** (when `--run-judge`).
  Per-(task, seed) winner + per-task BG win rate. Verdicts also
  written to `<output>.judge.jsonl`. See `bench/microbench_judge.py`.
- **Priority 2 — tokens.** Summary table shows per-rollout
  in_tok/out_tok; compare BG medians to raw using
  `bench/stats_aggregate.py::aggregate_per_cell` on the `.jsonl`.
  Regression threshold: +10% median input or output tokens.
- **Priority 3 — wall time.** Reported but weighted down —
  Gemma's thinking-mode inflates wall ~3× relative to cloud-API
  behaviour. Don't block commits on wall alone unless P1 or P2
  are also unfavourable.

For deeper post-hoc analysis:

```bash
bench/.venv/bin/python -c "
from bench.stats_aggregate import load_runs, aggregate_per_cell, arm_totals_with_ci
from pathlib import Path
runs = load_runs([Path('bench/runs/<YOUR_RUN>.jsonl')])
for (t, a), m in sorted(aggregate_per_cell(runs).items()):
    print(t, a, m)
"
```

## Known pitfalls (from six prior rounds)

- **Connection refused mid-run** → the model got evicted by llama-swap's TTL. Re-check step 4.
- **Empty content from Gemma** → thinking-mode `reasoning_content` field; set `max_tokens` ≥ 2048 and read `reasoning_content` not just `content`.
- **+320% input token regression** → the "fallback to bash after 2 BG calls" rule. If a new prompt tweak shows this pattern, revert immediately (see commit history).
- **BG arm using native `Read`** → the BG arm is hard-palette-restricted; if tokens spike check `BLASTGUARD_TOOLS` in `bench/microbench.py` for palette drift.

## After a clean run

1. Save the `.jsonl` under `bench/runs/` (gitignored) and summarise the aggregate in `docs/MICROBENCH.md` with a new dated row.
2. If the numbers regress from the previous row, do not commit the change that caused them — investigate first.
3. If the numbers improve, the commit message should cite the task and the before/after medians.
