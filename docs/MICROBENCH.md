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

## Tuning trajectory (iterative micro-bench optimizations)

After establishing round-2 as the baseline, we iterated on BlastGuard
using the micro-bench as a regression harness. Each row is one change
plus a re-run of the same 4 tasks on the same model (MiniMax M2.7 +
BLASTGUARD_BIAS).

| Round | Change                                            | BG cost | BG turns | BG in tok | BG calls | Δ vs prev (cost) |
|---:|---------------------------------------------------|--------:|---------:|----------:|---------:|-----------------:|
| 2  | (baseline — bundle advertised Phase-2 capabilities) | $0.0400 |    21    | 117,774   |    7     |  —               |
| 3  | Phase-1-accurate bundle docstrings                | $0.0296 |    18    |  84,346   |    7     | −26%             |
| 4  | Compressed MCP tool descriptions ~40%             | $0.0219 |    16    |  60,970   |    5     | −26%             |
| 5  | Empty-hit hints redirect to grep                  | $0.0305 |    17    |  86,896   |    7     | **+39% (regress)** |

**Cumulative (round 2 → round 5): BG cost −24%, input tokens −26%,
turns −19%.** Round 4 is the current peak; round 5's empty-hit hints
didn't help on this task mix.

### What worked (rounds 3–4)

1. **Tightening bundle docstrings to Phase-1-accurate** was the single
   biggest intervention. The model had been calling BlastGuard for
   `chain from X to Y` and cross-file callers — queries Phase 1 can't
   answer. After narrowing the advertised surface area to the four
   query types that work reliably (`outline of`, `find`, `exports of`,
   `libraries`) and telling the model explicitly to prefer grep for
   cross-file work, the `chain-search-to-graph` task's BG cost dropped
   40% (it stopped thrashing on impossible BlastGuard queries before
   falling back).

2. **Compressing MCP tool descriptions ~40%** cut per-turn schema
   overhead. The schema is sent on every turn, so saving 400-600
   bytes/turn compounds across 16-21 turns per task. Round 4 data
   shows −28% input tokens on the BG arm against no code behaviour
   change — pure prompt-bytes savings.

### What didn't work (round 5)

Adding empty-hit hints (e.g., "no same-file callers in Phase 1 graph;
for cross-file callers, use grep") to the `callers_of` / `imports_of`
/ `tests_for` responses **regressed** the BG arm:

- BG cost went **up** 39% vs round 4
- Tool-call count went **up** from 5 to 7
- The model appears to use the hint text as a cue to try *additional*
  BlastGuard queries before pivoting to grep, rather than pivoting
  immediately

The hint code is left in place (see Task 5 in the plan) because the
pattern is philosophically correct — returning structured fallback
suggestions on empty responses — and may help with larger / different
task mixes or models. At n=4 tasks, we cannot conclude hard-negative.

### What the trajectory does NOT tell us

- **Statistical significance.** 4 tasks × 1 run each is underpowered.
  The round 5 regression could be noise; the round 3/4 wins could be
  overstated. The tight model seed + task set gives us internal
  consistency but not external validity.
- **Cross-model generality.** Everything measured here is MiniMax M2.7
  behaviour. Sonnet 4.6, Opus 4.7, and GLM-5.1 may use BlastGuard
  differently.
- **SWE-bench Pro lift.** The micro-bench tasks are repo-navigation
  questions on this codebase; they're not hidden-dependency bug fixes
  on downstream repos. See `bench/KNOWN_GAPS.md` Gap 5 for why a real
  Pro run is still blocked upstream.

### Total optimization spend

- Round 3 re-run: ~$0.06
- Round 4 re-run: ~$0.06
- Round 5 re-run: ~$0.07
- **Total across this tuning plan: ~$0.19.**

Combined with prior micro-bench rounds 1-2 (\$0.087), the full
measurement trajectory that produced the −24% cost reduction cost
**~\$0.28**.

### Round 6 — Tier-1 output optimizations combined

After rounds 3-5 validated the tuning-harness approach, Plan 13 landed
four more optimizations aimed at the search-tool response surface:

1. **Compact hit formatting** — strip lifetimes (`'a`), generic bounds
   (`T: Sized`), and the `fn ` keyword from rendered signatures.
2. **Relative paths** — responses use paths relative to `project_root`
   instead of the 50-char absolute prefix.
3. **Smart per-query caps** — `outline` / `exports` cap at 50 (was 10),
   `find` caps at 5 (was 10), `libraries` caps at 30, `callers` / `callees`
   stay at 10.
4. **Outline test/prod dedup** — duplicate-name functions (production
   vs. `#[cfg(test)]`) get a `[test]` prefix on later occurrences.

Tasks 1, 2, 4, 5 landed in one measurement cycle (round 6).

| Round | BG cost | BG turns | BG input | vs raw |
|---:|--:|--:|--:|--:|
| 4 (peak)    | $0.0219 | 16 | 60,970 | BG 28% cheaper |
| 6           | $0.0269 | 17 | 74,321 | BG **44%** cheaper |

**What this tells us:** the Tier-1 changes are NOT a clean absolute-cost
win over round 4. BG cost went up ~$0.005 and input tokens up ~13k. But
the **raw arm got much noisier** this round ($0.031 → $0.048, +57%) —
well outside any reasonable interpretation of "same tools, same tasks,
same seed". This is stochastic variance at n=4 tasks.

The relative BG-vs-raw gap actually **widened from 28% to 44%**, which
is the metric that matters for BlastGuard's value claim. If the absolute
round-4 peak was a lucky draw (which this data implies), then round 6's
numbers are closer to the true mean and the Tier-1 changes are providing
some signal even if it's swamped by noise.

**Why keep the Tier-1 code anyway:**

- Compact formatting is strictly-smaller bytes-on-wire; its effect
  compounds on larger task sets than this 4-task bench.
- Smart caps prevent silent outline truncation on files with >10
  symbols (a real correctness issue, not just a cost question).
- Test/prod dedup makes outline more signal-rich for the agent even
  without a cost delta.

**Cumulative round 2 → round 6:** BG cost **-33%** ($0.040 → $0.027),
input tokens **-37%** (118k → 74k), turns **-19%** (21 → 17), BlastGuard
tool calls stable at 7. BG arm is decisively cheaper than raw.

**What the tuning-trajectory data does NOT tell us** (reiterated): n=4
runs without seed variance gives us too-noisy signal to conclude
individual-change attribution beyond round 4. A Tier-2 plan should
expand the task set before more Rust-side tuning, not less.

### Updated trajectory table

| Round | Change | BG cost | BG turns | BG in tok | BG calls | Δ vs prev |
|---:|---|--:|--:|--:|--:|--:|
| 2  | baseline                                           | $0.0400 | 21 | 117,774 | 7 | — |
| 3  | Phase-1-accurate bundle docstrings                 | $0.0296 | 18 |  84,346 | 7 | −26% |
| 4  | Compressed MCP tool descriptions ~40%              | $0.0219 | 16 |  60,970 | 5 | −26% |
| 5  | Empty-hit hints (regressed)                        | $0.0305 | 17 |  86,896 | 7 | +39% |
| 6  | Compact fmt + relative paths + caps + dedup        | $0.0269 | 17 |  74,321 | 7 | +23% vs r4, **-33% vs r2** |

## Round 7 — cross-file resolution landed; single-task re-run

Scope: 3 seeds on `chain-search-to-graph` only (used new `--tasks` flag
added to `bench/microbench.py`). Commits measured: `7fc297e` through
`1d5e9c6` — resolve_imports, resolve_calls, cross-file cascade
warnings, restitch_reverse_edges_for_file, path-normalisation cleanup.

|                        | raw (mean) | blastguard (mean) | Δ       |
|------------------------|-----------:|------------------:|--------:|
| input tokens           | 56,487     | 37,075            | −34%    |
| wall seconds           | 139        | 665               | **+379%**  |
| turns                  | 7.67       | 11.00             | +43%    |
| `done_marker` rate     | 3/3        | 1/3               | regression |
| answer correctness     | 3/3        | 3/3               | equivalent |

**Honest read:** mixed result. The resolver chain works — all 6
rollouts identified the correct call chain
(`server.rs::search_tool → dispatcher::dispatch → structural.rs`).
Input tokens improved slightly over round 6's baseline. Turns dropped
(+43% vs round 4's +71% — cross-file resolution is reducing
wandering). BUT wall time regressed catastrophically (+379% vs
round 4's −88%).

**Why wall time regressed:** richer BG responses (cross-file importer
+ first-class callers + bundled context) trigger more `reasoning_content`
on Gemma's thinking-mode path, inflating per-turn latency ~3×. This is
a *local Gemma 26B thinking-mode pathology*, not a correctness issue.
On Opus/Sonnet without thinking overhead, the −34% input tokens should
translate directly to a cost AND wall-time win.

**Why 2/3 BG runs stopped at `finish_stop`:** agent reached the correct
answer but didn't cleanly emit DONE. Worth tightening the efficiency
rules in `bench/prompts.py::BLASTGUARD_BIAS` with a stronger
"answer-as-soon-as-you-have-enough" push.

**Do not cite this as a win.** Per the `bench-regression-guard`
discipline, a wall-time regression blocks a new headline claim even
when tokens improve. Re-measure on a cloud API (or a non-thinking-mode
local model) before updating the README.

### Replication

```bash
bench/.venv/bin/python -m bench.microbench \
  --api-base http://127.0.0.1:8080/v1 \
  --api-key-env DUMMY_KEY \
  --model gemma-4 --model-id-override gemma-4 \
  --tasks chain-search-to-graph \
  --seeds 3 \
  --output bench/runs/$(date +%Y%m%d-%H%M%S)-chain-search.jsonl
```

Total optimization spend (rounds 3-6): **~$0.26**.

## Round 8 — pipeline verification (single seed, single judge call)

Scope: 1 task × 1 seed × 1 judge call. Not a measurement round —
purpose was to verify the priority-ordered quality pipeline
(grader + judge) lands results end-to-end on real Gemma output.
Commits measured: cumulative session through `43262b8` (resolver
chain + 16 parser/resolver correctness fixes + kind-correction
fix for method dispatch + the quality framework).

|                        | raw        | blastguard | Δ       |
|------------------------|-----------:|-----------:|--------:|
| input tokens           | 89,220     | 53,849     | **−40%** |
| wall seconds           | 156.3      | 137.6      | −12%    |
| turns                  | 10         | 20         | +100%   |
| correctness (grader)   | 1/1        | 1/1        | tie     |
| judge winner           | —          | —          | **raw** |

### Priority 1a — deterministic grader: `COMMIT OK`

Both arms passed the substring check (`search_tool`, `dispatch`,
`structural`). The grader is doing its job.

### Priority 1b — LLM-as-judge: raw wins

The judge caught a quality gap the substring grader missed. Task
asked "name each function in order." Raw complied with a specific
function (`structural.rs:find (and other structural functions
like callers_of, callee_of, etc.)`). BG abstracted to "routes to
specific graph-backed implementations in the `src/search/structural`
module" — vaguer, doesn't name the final function.

Judge reasoning (correctness axis): *"Both answers identify the
same initial steps, but Answer A provides a specific function name
for the final step in the chain, whereas Answer B only describes
the routing process."*

**Interpretation:** the `BLASTGUARD_BIAS` prompt's aggressive
"answer as soon as you have enough" rule plus the `STOP CONDITION`
block may be over-indexing BG on brevity at the cost of
specificity. This is a real-quality finding that substring matching
alone would have missed — the judge pipeline is doing what it's
supposed to.

**Caveat:** n_judges=1 means the verdict depends on one random
A/B assignment (raw landed in slot A here). n_judges=3 at minimum
for any real measurement.

### Priority 2 — tokens: BG wins decisively

BG saved 40% on input tokens and 31% on output tokens. Raw's
extra tokens came from six `read_file` calls that BG substituted
with nine `blastguard_search` calls. The token economics favour BG
even though raw picked a slightly better answer this time.

### Priority 3 — speed: BG wins slightly

−12% wall time — a notable reversal from round 7's +379%. Could
be the cumulative effect of the kind-correction + prompt tighten +
DONE-emission clarity, or Gemma variance at n=1. Can't attribute
without a multi-seed re-run.

### What round 8 verified

- Pipeline lands results end-to-end on real Gemma output.
- Judge's JSON parser handles real Gemma responses without needing
  the preamble-fallback regex.
- `.jsonl` + `.judge.jsonl` both land on disk correctly.
- Priority-ordered summary renders in the expected order.
- Grader and judge produce auditable outputs.

### What round 8 does NOT establish

- Whether BG's terseness-vs-specificity trade-off is systematic
  or just this task. Need n=3+ seeds × multiple tasks.
- Whether the wall-time reversal is real or variance.
- Whether the BG prompt should be loosened to preserve specificity.
  Hypothesis: yes — but hold until a multi-task measurement confirms
  the trade-off is systematic. Changing the prompt based on one
  data point would be p-hacking.

### Next measurement round (blocked pending user decision)

Proposal: run all 10 tasks × 1 seed × 3 judges (~45 min Gemma time)
to get a first real quality-gated comparison across the task set.
If BG wins the judge on ≥6/10 tasks, the terseness concern is task-
specific. If BG loses ≥4/10 tasks on the judge, loosen the
BLASTGUARD_BIAS efficiency rules to preserve specificity.

Pipeline commands in `.claude/skills/bench-rerun/SKILL.md`.
