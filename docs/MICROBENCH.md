# Micro-benchmark — 2026-04-19

Two rounds of paired A/B testing against the BlastGuard repo, using
MiniMax M2.7 via OpenRouter. The second round (with 4 tasks and
`BLASTGUARD_BIAS` applied to the BG arm) is what you should trust;
the first round (no bias, 2 tasks) is kept for comparison.

Not statistically powered (n=4). Purpose is to replace "projected"
with actual measurements on one concrete model + codebase.

## Setup

- **Model:** `minimax/minimax-m2.7` (OpenRouter, $0.30 / $1.20 per M tokens)
- **Project:** this repo (`/home/adam/Documents/blastguard`)
- **Arm A (raw):** tools `read_file`, `grep`, `bash` (Python implementations)
- **Arm B (blastguard):** same tools + `blastguard_search`,
  `blastguard_apply_change`, `blastguard_run_tests` (over the MCP bridge)
  — **plus the `BLASTGUARD_BIAS` steering prompt appended to the
  system message.**
- **Runner:** `bench/microbench.py`, max 25 turns per task.

Raw per-run logs: `bench/results/microbench.jsonl`.

## Tasks

1. **explore-cold-index** (easy — outline query)
2. **callers-apply-edit** (easy — direct grep target)
3. **chain-search-to-graph** (hard — multi-hop cross-file navigation)
4. **cascade-signature-change** (hard — blast-radius / dependency question)

## Round 2 results — with `BLASTGUARD_BIAS`

| Task | Arm | Turns | In tokens | Out tokens | Cost (USD) | Wall | BlastGuard calls |
|---|---|--:|--:|--:|--:|--:|:--|
| explore-cold-index       | raw        |  3 | 10,975 |   622 | $0.0040 | 15.7s | — |
| explore-cold-index       | blastguard |  3 |  8,341 |   497 | $0.0031 | 12.8s | **2** |
| callers-apply-edit       | raw        |  2 |  3,555 |   582 | $0.0018 | 13.0s | — |
| callers-apply-edit       | blastguard |  3 |  8,771 | 1,073 | $0.0039 | 21.7s | **1** |
| chain-search-to-graph    | raw        |  6 | 51,481 | 1,080 | $0.0167 | 29.7s | — |
| chain-search-to-graph    | blastguard | 10 | 81,026 | 1,269 | $0.0258 | 35.6s | **1** |
| cascade-signature-change | raw        |  3 | 12,657 | 1,164 | $0.0052 | 15.2s | — |
| cascade-signature-change | blastguard |  5 | 19,636 | 1,066 | $0.0072 | 14.4s | **3** |
| **TOTAL**                | **raw**        | **14** | **78,668** | **3,448** | **$0.0277** | **73.6s** | — |
| **TOTAL**                | **blastguard** | **21** | **117,774** | **3,905** | **$0.0400** | **84.5s** | **7** |

## What the data says — honestly

**The BlastGuard arm used more resources overall.** +50% turns, +44%
total cost, +15% wall time. This is the opposite of what we hoped for.

Task-by-task:

- **explore-cold-index:** BlastGuard **won** (24% fewer tokens, 23% less
  cost, 18% faster). `outline of src/index/indexer.rs` + `find cold_index`
  replaced a larger grep + read combo. This is BlastGuard's sweet spot.
- **callers-apply-edit:** BlastGuard **lost** (2.2× more cost, 1.7× slower).
  For a specific-symbol question, `grep 'apply_edit(' src/` was strictly
  better than `find apply_edit` + follow-up. BlastGuard's graph is
  intra-file only in Phase 1 (see README "Known limitations"), so for
  cross-file callers the model correctly needs to fall back.
- **chain-search-to-graph:** BlastGuard **lost hard** (+55% cost, +67% more
  turns). The agent called `blastguard_search` once early, got incomplete
  data (Phase 1 chain queries don't cross file boundaries), then pivoted
  to bash + read_file — but by then it had already invested tokens in
  the failed BlastGuard attempt. Net: BG arm did MORE exploration, not
  less.
- **cascade-signature-change:** BlastGuard **lost** (+38% cost, +67% more
  turns). Model called `blastguard_search` 3 times trying to find cross-file
  callers. Same Phase 1 limitation — it can't.

## What BlastGuard actually is, according to this data

Phase 1 BlastGuard, in aggregate on these 4 tasks with MiniMax M2.7 and
the `BLASTGUARD_BIAS` prompt:

- **Net cost:** higher than native-only. The tool-schema overhead (~1 KB
  per turn) is real, and when BlastGuard queries return partial data the
  model burns extra turns falling back.
- **Per-task:** wins cleanly on in-file outline/find, loses on cross-file
  dependency questions because Phase 1 doesn't resolve those edges.

**This is consistent with the published README** (which calls cross-file
calls a Phase 2 item) and with **CodeCompass's finding** (cited in
`README.md`): retrieval tools show **+20pp on hidden-dependency tasks**
only when the retriever can actually answer hidden-dependency questions.
BlastGuard Phase 1 cannot — yet.

## What this means for the v0.1.0 release

- The **projected +1 to +3 pp lift** in the README is *not* supported by
  this micro-bench. It's also not *refuted* — the tasks here aren't
  SWE-bench Pro tasks, and MiniMax M2.7 with `BLASTGUARD_BIAS` is one
  specific scaffold.
- The honest framing: **BlastGuard Phase 1 provides correct intra-file
  graph retrieval and cascade warnings; its end-to-end value on real
  refactor tasks depends on Phase 2 cross-file edges landing, and that's
  gated on exactly this kind of benchmark signal.**
- The bundle docstrings (`bench/bundles/blastguard/config.yaml`) should
  probably be tightened to NOT advertise `chain from X to Y` or cross-file
  callers, since the agent reaches for them and gets burned.

## Round 1 results — no bias (kept for comparison)

With no `BLASTGUARD_BIAS` in the system prompt, M2.7 **didn't call any
BlastGuard tools**. Both arms used native grep+read_file identically.
The small token/cost deltas in that run were stochastic.

Round 1 cost: $0.019 across 4 runs (2 tasks × 2 arms).

## Key finding vs. CodeCompass

CodeCompass predicts:
- **+20pp** on hidden-dependency tasks
- **0pp** on semantic/algorithmic tasks

Our 4 tasks break down as:
- 1 outline task (semi-hidden-dependency, intra-file) → BlastGuard wins
- 1 direct-symbol task (semantic) → BlastGuard loses
- 2 cross-file-dependency tasks (hidden-dependency) → BlastGuard should
  win per CodeCompass but **loses** because Phase 1 doesn't resolve
  cross-file edges

**This is a specific, actionable finding**: the Phase 2 cross-file
resolver isn't optional polish — it's the gate on BlastGuard's core
value proposition being *measurable*.

## Total spend

- Round 1: $0.019
- Round 2: $0.0677
- **Total: $0.087** for 12 paired runs

## Next experiments worth running

1. **Tighten bundle docstrings** so the agent doesn't attempt cross-file
   queries the Phase 1 graph can't answer. Re-run round 2 and check
   whether fewer false BlastGuard attempts reduces the cost gap.
2. **Phase 2 cross-file resolver** — implement and re-run. This is the
   claim test.
3. **Harder / more cross-file-rich tasks** from a real SWE-bench subset
   (once the Docker image tag blocker is resolved — see
   `bench/KNOWN_GAPS.md`).
4. **Other models.** M2.7 is one datapoint. Sonnet 4.6 / Opus 4.7 may
   use tools differently (Anthropic models are known for tool-call
   discipline).

## Replication

```bash
export OPENROUTER_API_KEY=sk-or-v1-...
cargo build --release
bench/.venv/bin/python -m bench.microbench
# Results in bench/results/microbench.jsonl
```
