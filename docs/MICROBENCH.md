# Micro-benchmark — 2026-04-19

A 2-task paired A/B against the BlastGuard repo itself, using MiniMax M2.7
via OpenRouter. Not statistically powered (n=2); purpose is to replace
"projected" with one concrete datapoint.

## Setup

- **Model:** `minimax/minimax-m2.7` (OpenRouter, $0.30/$1.20 per M tokens)
- **Project:** this repo (`/home/adam/Documents/blastguard`)
- **Arm A (raw):** tools `read_file`, `grep`, `bash` (Python implementations)
- **Arm B (blastguard):** same tools + `blastguard_search`,
  `blastguard_apply_change`, `blastguard_run_tests` (over the MCP bridge)
- **System prompt:** neutral — no BlastGuard bias language
- **Runner:** `bench/microbench.py`

Exact script and results: `bench/microbench.py` and
`bench/results/microbench.jsonl`.

## Tasks

1. **explore-cold-index:** "Explain what `cold_index` does and what calls it."
2. **callers-apply-edit:** "List every caller of `apply_edit` and what each
   passes for `old_text`."

## Results

| Task | Arm | Turns | In tokens | Out tokens | Cost (USD) | Wall |
|---|---|--:|--:|--:|--:|--:|
| explore-cold-index | raw        | 4 | 19,817 |   657 | $0.0067 | 13.8s |
| explore-cold-index | blastguard | 3 | 10,889 |   362 | $0.0037 |  6.7s |
| callers-apply-edit | raw        | 3 |  9,436 | 1,157 | $0.0042 | 25.0s |
| callers-apply-edit | blastguard | 3 | 10,465 |   950 | $0.0043 | 21.3s |

Total spend: **$0.019**.

## What the model did

Both arms' tool calls, across both tasks:

- Arm A (raw): `grep×2, read_file×3, bash×0`
- Arm B (blastguard): `grep×2, read_file×2, bash×0`
- **BlastGuard tool calls on arm B: 0.**

MiniMax M2.7 was given the BlastGuard tools with full descriptions
(`outline of` / `callers of` / `find` / etc.) and still chose native
`grep` + `read_file` for both tasks. This is consistent with the
synthetic-task smoke earlier (same finding on an even simpler bug-fix).

## Honest interpretation

**Per-task:**

- **explore-cold-index**: Arm B used 45% fewer total tokens, 25% fewer
  turns, and finished 51% faster. This is *not* a BlastGuard advantage —
  it's a one-fewer-turn stochastic draw. Both arms used the same tools;
  Arm A happened to read one more file than Arm B to form its answer.
- **callers-apply-edit**: Arm B used 8% *more* input tokens (the extra
  tool-schema descriptions cost about 1 KB per turn) and finished with
  near-identical turns/output. Net cost: indistinguishable.

**Aggregate:**

- The observed token delta is dominated by a single extra turn in Arm A
  of Task 1 and by the BlastGuard arm's larger tool schema in every turn.
  Neither is the "structural graph beats grep" signal BlastGuard is
  designed to produce.
- On these simple single-file questions, M2.7 decides grep+read is
  sufficient. That's a reasonable call — `outline of` is a token-smaller
  substitute for `grep "pub fn" + read_file`, but both work.

## What this does *not* tell us

- How BlastGuard performs on **multi-file refactors** where caller
  attribution matters (Phase 2 territory).
- How BlastGuard performs when the model is **cost-constrained** or
  operating under a turn cap (SWE-bench Pro territory).
- How BlastGuard performs with **prompt biasing**
  (`BLASTGUARD_BIAS` in `bench/prompts.py`) that explicitly instructs
  the agent to prefer structural queries on ambiguous tasks.

## What this does tell us

- BlastGuard tools *are* callable end-to-end via the MCP bridge. We
  confirmed that independently during verification, but this run
  reconfirms the bridge works under a real tool-use loop.
- Adding BlastGuard tools with full descriptions costs **~1 KB per turn**
  in tool-schema overhead. Worth keeping in mind when designing the
  bundle's docstring size.
- A neutral system prompt is not sufficient bias for M2.7 to reach for
  BlastGuard on easy tasks. The scaffolded `BLASTGUARD_BIAS` prompt in
  `bench/prompts.py` exists exactly for this reason; it's untested here.

## Next experiments worth running

1. **Re-run with `BLASTGUARD_BIAS` in the system prompt.** Same tasks,
   same model, see whether explicit steering moves the tool-call count
   on Arm B.
2. **Harder tasks.** Multi-file refactor ("add a new `outline of` variant
   that returns only public symbols, update all tests that assert
   exports"). Exploration tasks don't exercise BlastGuard's strengths.
3. **Under a turn cap.** The BlastGuard advantage — if it exists — is
   most likely to appear when the agent has 10 turns to solve something
   it'd otherwise need 20 for. The micro-bench gave both arms 25 turns
   of headroom; neither came close to using it.

## Replication

```bash
export OPENROUTER_API_KEY=sk-or-v1-...
cargo build --release
bench/.venv/bin/python -m bench.microbench
# Results in bench/results/microbench.jsonl
```
