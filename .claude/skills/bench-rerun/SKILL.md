---
name: bench-rerun
description: Run a Gemma-4 microbench task (or the full suite) against BlastGuard with the strict setup ritual. Use when you want to measure the impact of a recent change on agent efficiency.
disable-model-invocation: true
---

# Bench Rerun (Gemma-4 26B, local, zero API cost)

User-only: Claude will not invoke this automatically. Type `/bench-rerun` or `/bench-rerun <task_id>` to run.

## Setup ritual (every run)

All steps matter — cutting any one of them produced bad data last time:

1. **Ensure llama-swap is up and serving Gemma-4.**
   ```bash
   curl -sS -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8080/v1/models
   ```
   Expect `200`. If you get anything else, start llama-swap before continuing.

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

5. **Confirm the model is loaded with a 32K context.** `-c 8192` truncates prompts mid-rollout. `-c 32768` is correct.

## Run

### Single task (fast feedback)

```bash
python bench/microbench.py \
  --api-base http://127.0.0.1:8080/v1 \
  --api-key-env DUMMY_KEY \
  --model-id-override gemma-4-27b-it \
  --tasks <task_id> \
  --seeds 3 \
  --arms raw blastguard \
  --out bench/runs/$(date +%Y%m%d-%H%M%S)-<task_id>.jsonl
```

Typical tasks: `explore-cold-index`, `chain-search-to-graph`, `find-symbol`, `tests-for-symbol`, `apply-edit-cascade`.

### Full suite (slow — 10–20 min)

Drop `--tasks` to run every task in `bench/tasks_registry.py`.

## Analyse

```bash
python bench/stats_aggregate.py bench/runs/<YOUR_RUN>.jsonl
```

Compare against the last committed baseline in `docs/MICROBENCH.md`. Key fields to watch:

- **Input tokens (median)** — should not regress by >10% on any task.
- **Wall-clock seconds (median)** — the headline efficiency number.
- **Turn count (median)** — more is OK only if each turn is cheaper.
- **Paired McNemar's `p` on exit_status=submitted** — needs to be < 0.1 to claim a win.

## Known pitfalls (from six prior rounds)

- **Connection refused mid-run** → the model got evicted by llama-swap's TTL. Re-check step 4.
- **Empty content from Gemma** → thinking-mode `reasoning_content` field; set `max_tokens` ≥ 2048 and read `reasoning_content` not just `content`.
- **+320% input token regression** → the "fallback to bash after 2 BG calls" rule. If a new prompt tweak shows this pattern, revert immediately (see commit history).
- **BG arm using native `Read`** → the BG arm is hard-palette-restricted; if tokens spike check `BLASTGUARD_TOOLS` in `bench/microbench.py` for palette drift.

## After a clean run

1. Save the `.jsonl` under `bench/runs/` (gitignored) and summarise the aggregate in `docs/MICROBENCH.md` with a new dated row.
2. If the numbers regress from the previous row, do not commit the change that caused them — investigate first.
3. If the numbers improve, the commit message should cite the task and the before/after medians.
