---
name: bench-quick
description: Run a ~10-minute 3-task × 1-seed smoke bench that catches the regressions worth catching in the inner dev loop — tool-call plumbing, BG palette health, structural-query performance. Use when the user wants a fast sanity check during active development, says "quick bench", "smoke check", "is BG still winning X", or just landed a harness/prompt/tool-palette change and wants a fast-read signal before committing. This is NOT a replacement for the full 10-task × 3-seed rigor bench — it's the dev-loop inner cycle that catches the ~80% of regressions that matter while shipping.
disable-model-invocation: true
---

# Quick Bench (~10 min, 3 canary tasks, n=1)

User-only. Type `/bench-quick` to run. Designed for the active-development inner loop. For a rigor round before release, use the full bench via `.claude/skills/bench-rerun/SKILL.md`.

## What this is

3 canary tasks × 1 seed × 2 arms = 6 rollouts, grader-only (no judge).

**Runtime estimates:**
- Gemma 4 26B A4B on llama-swap: ~6 min
- Qwen 3.6 35B A3B jinja: ~15-20 min (slower due to unlimited reasoning)
- Cloud APIs (Sonnet 4.6 / Opus 4.7): ~5 min

**Goal:** catch regressions that would otherwise go unnoticed between rigor rounds — model config bugs, prompt typos, palette breakage, bench harness drift.

## Canary tasks (deliberate selection)

| task | what it probes |
|---|---|
| `find-tamper-patterns` | Basic tool-call plumbing. Both arms should pass. Fails fast if the chat template doesn't parse tool calls (today's Qwen `--jinja`-missing bug would have surfaced in 2 min instead of 25 min) |
| `callers-apply-edit` | BG's strongest Gemma cost-win (−87% input tokens). If BG doesn't clearly beat raw on tokens here, the BG palette has a regression |
| `outline-tree-sitter-rust` | Structural-query sweet spot (`outline of PATH`, `exports of PATH`). Tests the BG response-format layer |

**Explicitly excluded:** chain-search-to-graph, impact-of-removing-libraries, trace-cache-persistence. Multi-hop tasks; 5-10 min per rollout on reasoning models. Slow + variance-heavy = wrong fit for the inner loop.

## What this does NOT do

- **No LLM-as-judge.** Save 5-10 min. If you want the P1b verdict, use `/bench-rerun` for the full suite or `/tmp/run_judge_only.py` standalone on the emitted .jsonl afterwards.
- **No multi-seed.** Single sampling draw per cell. That's OK for a smoke — a regression on n=1 is suggestive; a regression at n=3 is confirming. Promote to rigor round if something looks off.
- **No cache flush between arms.** Intentionally — cache contamination doesn't matter at this scale because we're looking for qualitative regressions, not token-delta precision.

## Setup ritual (short)

1. Confirm your target llama-server is running + responsive.
2. Clear BlastGuard's index cache (`.blastguard` dir) — ensures cold-index measurement.
3. Build release if touched (`cargo build --release 2>&1 | tail -3`).

## Run

### Against currently-running Qwen 3.6 on port 8001

```bash
test -d .blastguard && rm -r .blastguard
QWEN_KEY=sk-local-codex bench/.venv/bin/python -u -m bench.microbench \
  --api-base http://127.0.0.1:8001/v1 --api-key-env QWEN_KEY \
  --model qwen3.6 --model-id-override qwen3.6 \
  --tasks find-tamper-patterns,callers-apply-edit,outline-tree-sitter-rust \
  --seeds 1 \
  --output bench/runs/$(date +%Y%m%d-%H%M%S)-quick-qwen.jsonl
```

### Against Gemma 4 on port 8080 (llama-swap)

```bash
test -d .blastguard && rm -r .blastguard
DUMMY_KEY=sk-local bench/.venv/bin/python -u -m bench.microbench \
  --api-base http://127.0.0.1:8080/v1 --api-key-env DUMMY_KEY \
  --model gemma-4 --model-id-override gemma-4 \
  --tasks find-tamper-patterns,callers-apply-edit,outline-tree-sitter-rust \
  --seeds 1 \
  --output bench/runs/$(date +%Y%m%d-%H%M%S)-quick-gemma.jsonl
```

### With watchdog (stall = kill after 5 min)

Prepend:
```bash
bench/watchdog.sh bench/runs/<output>.jsonl 300 &
```
…before the microbench command.

## Interpret

The `=== SUMMARY ===` and `=== QUALITY (Priority 1a — substring grader) ===` blocks print at end-of-run.

**Green signals:**
- Both arms produce non-empty answers (`stopped_reason=done_marker` > 4/6).
- BG grader ≥ raw grader on callers-apply-edit (BG is expected to dominate here).
- BG input tokens < raw input tokens on outline-tree-sitter-rust (BG's structural-query win).

**Red signals:**
- >2 rollouts end in `finish_stop` with empty `final_answer` → tool-call parsing broken (check `--jinja` flag on the server).
- BG grader < raw grader on all 3 tasks → palette regression; investigate BLASTGUARD_BIAS or tool schema.
- Both arms fail the same task → task prompt or harness issue, not a BG issue.

## When NOT to use this

- You need substance-vs-correctness axis breakdown (requires judge → use full bench).
- You need statistical significance for per-task claims (requires n≥3 → use full bench).
- You just changed something that only manifests on multi-hop tasks (chain-search etc.) — quick-bench won't exercise those.
- You're on cloud API and every token costs money — run once a day, not every commit.

## After a clean run

If green: commit. No need to write up a quick-bench run in `docs/MICROBENCH.md` — that's for rigor rounds only.

If red: inspect the offending rollout's `final_answer` and `tool_calls` fields in the .jsonl. Most common causes are server config (missing flag), prompt typo (breaking something the agent relied on), or a palette field rename.
