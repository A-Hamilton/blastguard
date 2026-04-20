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

5. **Context window vs VRAM — `-c 32768` is the working size on 17GB cards ONLY when KV-quant is on (see step 6).** History:
   - Round 9 attempt 1 crashed at `-c 32768` WITHOUT KV-quant (kernel OOM, not VRAM).
   - Rounds 9–11 ran at `-c 16384` with KV-quant on — stable but tight.
   - Round 12 raw arm hit a context-overflow 400 at `-c 16384` when `read_file` dumped a long source file (26446-token request vs 16384-token ctx), mid-rollout. With KV-quant on, `-c 32768` fits comfortably (VRAM peaked ~10GB of 17GB during that round).
   - **Current recommendation for 17GB cards: `-c 32768` + `-ctk q4_0 -ctv q4_0`.** Don't go to `-c 32768` without the KV-quant — round-9's OOM will return.
   - If the card is ≥24GB, `-c 32768` is fine with or without KV-quant.

6. **KV cache quantization is the single biggest RAM win** when combined with `--n-cpu-moe N`. Add `-ctk q4_0 -ctv q4_0` to llama-swap's Gemma command. Measured impact on round 9's first failure: RAM usage 18 GB → **5.8 GB** (−12 GB), swap 17 GB → **3.3 GB** (−14 GB), throughput −3 tok/s (negligible). Without this, `--n-cpu-moe 20` drives the system deep into swap, kernel OOM killer fires on llama-server. The fix is in `~/.config/llama-swap/config.yaml`:

   ```yaml
   cmd: |
     /usr/bin/llama-server
     -hf ggml-org/gemma-4-26B-A4B-it-GGUF
     -hff gemma-4-26B-A4B-it-Q4_K_M.gguf
     -ngl 99 --n-cpu-moe 20 -c 32768 -fa on
     -ctk q4_0 -ctv q4_0         # <-- KV cache quantization
     --host 127.0.0.1 --port ${PORT}
     --jinja
   ```

7. **If Gemma hangs mid-run**: check both VRAM (`rocm-smi --showmeminfo vram`) AND RAM (`free -h`). Round 9's first failure was RAM-OOM, not VRAM — `journalctl --user -u llama-swap` showed `Failed with result 'oom-kill'` from the kernel OOM killer. If swap is saturated, the fix is usually KV-cache quantization (step 6) or reducing `--n-cpu-moe`. If VRAM is full, reduce `-c`. If both are low and requests still hang, llama-swap failed to reload — `systemctl --user restart llama-swap` fixes it.

8. **Kernel swappiness should be ≤ 100.** Round 9's first failure hit swap thrash partly because `/proc/sys/vm/swappiness` was 150 (abnormally aggressive). Default is 60. Check with `cat /proc/sys/vm/swappiness`; if >100, `sudo sysctl vm.swappiness=60`. Persist across reboots via `/etc/sysctl.d/99-swappiness.conf`.

9. **Reasoning mode on/off.** Gemma 4 has thinking-mode enabled by default (`--jinja` activates the chat template's `<think>` tags). Pros: potentially better reasoning on hard tasks. Cons: inflates output tokens, wall time, and empty-`content` responses on tight `max_tokens`. To disable, add `--reasoning off` to the llama-server command. For a baseline A/B, run the bench once with reasoning on, once off, compare quality/token/wall via the priority-ordered summary.

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
