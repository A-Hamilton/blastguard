# BlastGuard v0.1.0 — Open-Source Release Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship BlastGuard v0.1.0 publicly — push 129 local commits to origin, rewrite README to honestly frame what's measured vs. projected, tag the release, and document the roadmap so contributors know what's next.

**Architecture:** No code changes to `src/`. Documentation, honesty pass, git release. The benchmark infrastructure (`bench/`) stays in-tree with honest "infrastructure complete; run gated on upstream SWE-agent Docker bug" framing. Community can pick up the benchmark work or wait for upstream fixes.

**Tech Stack:** Markdown, git, GitHub Releases. No runtime changes.

---

## Why ship now

BlastGuard Phase 1 is a good open-source tool on its own. It provides real capabilities:

- AST-graph retrieval via `search` (outline / callers / find / exports — live verified)
- Cascade-warning-aware `apply_change`
- Test-failure attribution via `run_tests`
- stdio MCP server, drop-in for Claude Code
- 253 Rust library tests + integration tests + CI, clippy pedantic clean

The benchmark lift is not verified (SWE-agent Pro integration is blocked upstream). That's fine — the README frames it as projected-from-adjacent-research. The tool's value isn't gated on our own number.

What we lose by delaying: real-world feedback from users who might try it today. What we gain by shipping: that feedback.

---

## File structure

**Modify:**
- `README.md` — rewrite "Honest positioning" section; remove claims about a harness that currently can't produce numbers
- `CHANGELOG.md` — add `[0.1.0] 2026-04-19` stanza marking Phase 1 shipped
- `bench/README.md` — update "Current state" section to match reality (infrastructure complete, run blocked upstream)
- `bench/KNOWN_GAPS.md` — add SWE-bench Pro Docker tag truncation + path forward

**Create:**
- `ROADMAP.md` — Phase 2 items (cross-file call edges, semantic search, Go support, SWE-bench Verified/Pro runs)
- `docs/BENCHMARKING.md` — how the benchmark harness is wired and what's needed to run it end-to-end

**Git:**
- Push `main` to `origin/main`
- Tag `v0.1.0`
- (Manual, by user) create GitHub Release from the tag

---

## Task 1: Rewrite README "Honest positioning" section

**Files:**
- Modify: `README.md:79-110` (the Honest-positioning + Known-limitations sections)

- [x] **Step 1: Read the current README positioning language**

Run: `sed -n '79,110p' README.md`
Note what currently says "Measured SWE-bench Pro lift has not been published yet" and "the harness lives in bench/; see bench/README.md for the run command".

- [x] **Step 2: Replace the section**

Use `Edit` to replace:

```markdown
## Honest positioning

This is a Phase 1 MVP. **Measured SWE-bench Pro lift has not been published yet.**
The harness lives in `bench/`; see `bench/README.md` for the run command.

Projected lift is `+1 to +3 points` on SWE-bench Pro with a realistic confidence
interval of `-1 to +5` — grounded in peer-reviewed adjacent research:
```

with:

```markdown
## Honest positioning

This is a Phase 1 MVP. **BlastGuard's own SWE-bench lift has not been measured
yet** — the harness (`bench/`) is built and live-verified on synthetic tasks,
but a full benchmark run is currently blocked by an upstream bug in SWE-agent's
Docker deployment when consuming SWE-bench Pro images (tag-length truncation).
See `docs/BENCHMARKING.md` for the setup and `bench/KNOWN_GAPS.md` for the
blocker.

Projected lift is `+1 to +3 points` on SWE-bench Pro with a realistic confidence
interval of `-1 to +5` — grounded in peer-reviewed adjacent research:
```

- [x] **Step 3: Add a "What's verified today" bullet list right after the projections**

Use `Edit` to append, right before the "## Known limitations (Phase 1)" header:

```markdown
### What's verified today (not projected)

- Rust codebase: 253 library tests pass, clippy pedantic clean, `cargo fmt` clean.
- MCP handshake + all three tools live-tested against the release binary.
- `search` structural queries (outline / callers / find / exports / libraries)
  return correct results on this repo.
- `apply_change` error propagation and `run_tests` cargo auto-detect verified.
- SWE-agent integration: agent invokes BlastGuard tools via the bundle over
  real MCP (4 tool calls on synthetic task, exit_status=submitted, $0.01 spend).

What's pending: the lift number on SWE-bench Pro. Waiting on upstream
SWE-agent fixes or community contribution.
```

- [x] **Step 4: Run a sanity pass**

Run: `grep -n 'see `bench/README.md` for the run command' README.md || echo not found`
Expected: `not found` — the old "see bench/README for run command" line is gone.

- [x] **Step 5: Commit**

```bash
git add README.md
git commit -m "docs(readme): honest framing — infrastructure built, benchmark run gated upstream"
```

---

## Task 2: Update CHANGELOG with 0.1.0 stanza

**Files:**
- Modify: `CHANGELOG.md`

- [x] **Step 1: Read current CHANGELOG**

Run: `head -20 CHANGELOG.md`
Expected: shows `## [0.1.0] — Phase 1 MVP (unreleased)` header.

- [x] **Step 2: Rewrite the 0.1.0 header and prepend a stanza**

Use `Edit` to replace:

```markdown
## [0.1.0] — Phase 1 MVP (unreleased)
```

with:

```markdown
## [0.1.0] — 2026-04-19 — Phase 1 MVP

### Added (release summary)

- Phase 1 feature-complete MCP server. See the detailed list below for
  components added during Phase 1.
- Benchmark harness (`bench/`) with SWE-agent scaffold integration, paired-
  comparison orchestrator, McNemar's test, BenchJack tamper defense, and
  budget caps. Infrastructure is live-verified; an end-to-end SWE-bench Pro
  run is blocked by upstream Docker-tag handling in SWE-agent (see
  `bench/KNOWN_GAPS.md`).

### Known at release

- No published BlastGuard lift number on SWE-bench Pro yet. Projection
  remains +1 to +3 pp based on adjacent research (cAST, WarpGrep v2,
  Auggie, CodeCompass).
- Cross-file call edges are intra-file-only in Phase 1 — a documented
  limitation; see README.

---

## [0.1.0-infrastructure] — Phase 1 MVP (original)
```

- [x] **Step 3: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): 0.1.0 release stanza with honest benchmark status"
```

---

## Task 3: Write ROADMAP.md

**Files:**
- Create: `ROADMAP.md`

- [x] **Step 1: Write the roadmap**

```markdown
# BlastGuard Roadmap

## Phase 1 — Shipped (v0.1.0, 2026-04-19)

Stdio MCP server with three tools (`search`, `apply_change`, `run_tests`)
and full supporting infrastructure (AST graph, BLAKE3 Merkle cache, file
watcher, cascade detectors, benchmark harness). See `CHANGELOG.md`.

## Phase 2 — Post-benchmark (contingent)

These items are explicit trade-offs, not commitments. Land them once real
benchmark data indicates the return.

- **Cross-file call edges.** Phase 1 resolves `Imports` edges across files
  but keeps `Calls` edges intra-file. A resolved cross-file call graph
  unlocks cross-file caller queries and broader `apply_change` cascades.
  Gated on: evidence that cross-file retrieval moves the benchmark needle
  (CodeCompass predicts +20pp on hidden-dependency tasks).
- **Semantic search via sqlite-vec + fastembed.** Feature-gated
  (`--features semantic`) to keep the MVP binary slim. Adds `around X`
  bundled retrieval (SPEC §3.1.3). ~130 MB embedding model cost.
- **Go language support.** Driver + resolver. Deferred pending SWE-bench
  Pro run signal on whether Go tasks are where we lose points.
- **Additional cascade detectors.** PARAM_ORDER, VISIBILITY,
  REEXPORT_CHAIN, CIRCULAR_DEP. Land only once Phase-1 detector data shows
  they'd fire usefully.

## Benchmark pipeline

- **Status:** infrastructure complete (`bench/`), end-to-end verified on
  synthetic tasks with MiniMax M2.7, blocked on upstream SWE-agent Docker
  tag-length handling for real SWE-bench Pro images. See
  `bench/KNOWN_GAPS.md`.
- **Unblock paths:** (a) upstream patch to SWE-agent's swerex deployment,
  (b) local image-retagging preflight, (c) pivot to SWE-bench Verified
  where the schema ships with `image_name` natively and SWE-agent's HF
  loader works out of the box.
- **Success criterion:** paired McNemar's p < 0.05 with delta >= +1 pp on
  a published benchmark; token efficiency delta ≥ 20% (BlastGuard arm vs.
  raw arm).

## Contributing

- Read `CONTRIBUTING.md`.
- Small fixes: PR welcome.
- New language driver: open an issue with a short design note first.
- Benchmark work: coordinate on the GitHub issue thread — we want the
  numbers to be honest, not duplicated.
```

- [x] **Step 2: Commit**

```bash
git add ROADMAP.md
git commit -m "docs: add ROADMAP with Phase 2 + benchmark status"
```

---

## Task 4: Update bench/README.md "Current state"

**Files:**
- Modify: `bench/README.md`

- [x] **Step 1: Find the workflow section**

Run: `grep -n '## Workflow\|## Setup' bench/README.md | head`

- [x] **Step 2: Prepend a "Current state" section before "Workflow"**

Use `Edit` to insert, immediately after the file's top-level description and before the first `## Workflow` heading:

```markdown
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
```

- [x] **Step 3: Commit**

```bash
git add bench/README.md
git commit -m "docs(bench): current-state section — infrastructure built, run blocked upstream"
```

---

## Task 5: Add Docker-tag blocker to KNOWN_GAPS

**Files:**
- Modify: `bench/KNOWN_GAPS.md`

- [x] **Step 1: Read current content**

Run: `cat bench/KNOWN_GAPS.md`

- [x] **Step 2: Append a new "Gap 5" section**

Use `Edit` to append (don't replace existing content — the prior gaps are historically useful):

```markdown

## Gap 5 — SWE-agent Docker tag truncation (upstream, open)

SWE-agent's `swerex.deployment.docker.DockerDeployment._build_image()`
runs `docker build -q --build-arg BASE_IMAGE=<long_tag> -` as a step
in every task's environment setup. The `<long_tag>` is passed straight
through from our instance JSON's `image_name` field
(`jefzda/sweap-images:<dockerhub_tag>`).

For SWE-bench Pro, `dockerhub_tag` can run 100-130 characters
(`internetarchive.openlibrary-internetarchive__openlibrary-<40-char-sha>-v<40-char-sha>`).
Docker enforces a **128-character max on image tags**; anything longer
is silently truncated in the shell command. The truncated tag doesn't
exist on Docker Hub, the build fails with `CalledProcessError(1)`, and
the task never reaches the agent.

Related: `scaleapi/SWE-bench_Pro-os` issue #75 ("Inconsistency between
images on DockerHub and Dockerfiles in repo").

**Unblock options:**

1. Pre-process step: `docker pull jefzda/sweap-images:<long>` then
   `docker tag jefzda/sweap-images:<long> bench/pro:<short_hash>` for
   every task. Rewrite `instances.jsonl` to point at the short tag.
   Adds disk + pull time up front.
2. Upstream patch to SWE-agent: skip the `docker build` wrapper when the
   base image is already usable (`--use_local_docker` beta flag area).
3. Pivot to SWE-bench Verified: `princeton-nlp/SWE-bench_Verified` ships
   `image_name` natively with tags under the 128-char limit. All our
   scaffold code (bundle / bridge / compare.py / McNemar's) transfers
   without change.

Option 3 is the cheapest path to a first BlastGuard lift number.
```

- [x] **Step 3: Commit**

```bash
git add bench/KNOWN_GAPS.md
git commit -m "docs(bench): document SWE-agent docker tag truncation blocker"
```

---

## Task 6: Write docs/BENCHMARKING.md

**Files:**
- Create: `docs/BENCHMARKING.md`

- [x] **Step 1: Write the doc**

Content:

```markdown
# Benchmarking BlastGuard

This document explains the `bench/` harness architecture, how to run it,
and what's currently gating an end-to-end SWE-bench Pro result.

## What the harness does

A **paired** measurement of BlastGuard's lift on SWE-bench-style tasks.
Same model, same tasks, same seed; the only variable is whether the
agent has BlastGuard available as an MCP tool.

- **Arm A (raw):** SWE-agent with its default tool registry.
- **Arm B (BlastGuard):** identical scaffold plus a tool bundle exposing
  `blastguard_search`, `blastguard_apply_change`, and
  `blastguard_run_tests` over an MCP bridge.

Per-task outcomes get paired by `instance_id` and fed through
[McNemar's test](https://en.wikipedia.org/wiki/McNemar%27s_test)
(`bench/stats.py`) to distinguish real lift from run-to-run noise.

## Architecture

```
  HF Dataset              prepare_instances.py          batch_runner.py
  (ScaleAI/SWE-bench_Pro)  →  instances.jsonl  →  build_batch_config()
                                                         ↓
                                                   sweagent run-batch
                                                         ↓
                                                   preds.jsonl
                                                         ↓
                                                   evaluator.py (SWE-bench_Pro-os)
                                                         ↓
                                                   compare.py (McNemar's)
```

Budget, telemetry, and BenchJack tamper defense live in:
- `bench/budget.py` (post-hoc cost tracking)
- `bench/telemetry.py` (per-task JSONL writer)
- `bench/evaluator.py::detect_tampering` (conftest.py / workflow-file
  rejection; the tamper vectors are documented in the grader source)

## How to run (once upstream is unblocked)

Prerequisites: Docker, OpenRouter API key, HuggingFace token,
`bench/.sweagent-repo/` cloned via `bench/scripts/clone_sweagent.sh`,
`target/release/blastguard` built via `cargo build --release`,
`bench/.evaluator/` cloned via `bench/scripts/clone_evaluator.sh`.

```bash
cd /path/to/blastguard
export OPENROUTER_API_KEY="sk-or-v1-..."
export HF_TOKEN="hf_..."
export SWE_AGENT_CONFIG_DIR="$(pwd)/bench/.sweagent-repo/config"
export SWE_AGENT_TOOLS_DIR="$(pwd)/bench/.sweagent-repo/tools"
export SWE_AGENT_TRAJECTORY_DIR="$(pwd)/bench/.sweagent-repo/trajectories"
export SWEAGENT_BINARY="$(pwd)/bench/.venv/bin/python $(pwd)/bench/scripts/sweagent_with_pricing.py run-batch"

# 10-task paired smoke, ~$5
HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 10 --seed 42 --budget-usd 5 \
  --run-id smoke-raw --model openrouter/minimax/minimax-m2.7 \
  --per-task-cost-limit 0.50 --batch-timeout 3600

HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
  --arm blastguard --limit 10 --seed 42 --budget-usd 5 \
  --run-id smoke-bg --model openrouter/minimax/minimax-m2.7 \
  --per-task-cost-limit 0.50 --batch-timeout 3600 \
  --blastguard-binary "$(pwd)/target/release/blastguard"

# Grade
bench/.venv/bin/python -c "
from bench.evaluator import run_evaluator
from pathlib import Path
for r in ('smoke-raw', 'smoke-bg'):
    run_evaluator(
        evaluator_dir=Path('bench/.evaluator'),
        raw_sample_csv=Path('bench/.evaluator/swe_bench_pro_full.csv'),
        patches_json=Path(f'bench/results/{r}/patches.json'),
        output_dir=Path(f'bench/results/{r}/eval'),
    )"

# Compare
bench/.venv/bin/python -m bench.compare \
  --raw-output-dir bench/results/smoke-raw/eval \
  --blastguard-output-dir bench/results/smoke-bg/eval
```

## Why a run hasn't been published

See `bench/KNOWN_GAPS.md` Gap 5. The short version: SWE-agent's Docker
deployment truncates SWE-bench Pro image tags past 128 chars and every
task fails at environment setup. We've invested in three workarounds
(manual pricing registration, per-instance call caps, timeout-trajectory
rescue) but the truncation itself needs an upstream fix, a preflight
re-tagging pass, or a pivot to SWE-bench Verified.

## Guardrails that are live

- **Turn cap** (`bench/sweagent_runner.py::DEFAULT_PER_INSTANCE_CALL_LIMIT`):
  40 API calls per task.
- **Cost cap** (config `per_instance_cost_limit`): defaults to $0.50 per
  task when the model is LiteLLM-priced or carries manual pricing.
- **Run-level budget** (`bench/budget.py`): post-hoc check, raises
  `BudgetExceeded` if a record() call would cross the cap.
- **BenchJack defense** (`bench/evaluator.py::detect_tampering`):
  flags edits to `conftest.py`, `pytest.ini`, `pyproject.toml`,
  `setup.cfg`, `tox.ini`, or `.github/workflows/**`.
- **Infra-failure filter** (`bench/compare.py::pair_results`):
  excludes tasks where either arm hit an evaluator error, empty patch,
  or SWE-agent non-`submitted` exit from McNemar's counts.
```

- [x] **Step 2: Commit**

```bash
git add docs/BENCHMARKING.md
git commit -m "docs: add BENCHMARKING.md explaining harness architecture + blockers"
```

---

## Task 7: Final verification sweep

**Files:**
- None modified; verification only.

- [ ] **Step 1: Rust suite**

Run: `cargo test --lib 2>&1 | tail -3`
Expected: `test result: ok. 253 passed; 0 failed` (or current count).

- [ ] **Step 2: Clippy pedantic**

Run: `cargo clippy --all-targets -- -W clippy::pedantic -D warnings 2>&1 | tail -3`
Expected: last line starts with `Finished` (no warnings).

- [ ] **Step 3: cargo fmt check**

Run: `cargo fmt --check`
Expected: exits 0 silently.

- [ ] **Step 4: Python tests**

Run: `cd bench && HF_HOME=/tmp/hf uv run pytest 2>&1 | tail -3`
Expected: `29 passed` (or current count).

- [ ] **Step 5: ruff**

Run: `cd bench && uv run ruff check 2>&1 | tail -3`
Expected: `All checks passed!` or `n/a`.

- [ ] **Step 6: Stop here if anything failed**

If any verification step above failed, STOP and fix before proceeding. Do not push a broken release.

---

## Task 8: Push to origin and tag v0.1.0

**Files:**
- None — git operations only.

- [ ] **Step 1: Confirm branch + ahead count**

Run: `git status && git rev-list --count origin/main..HEAD`
Expected: on `main`, clean working tree, integer count of commits ahead.

- [ ] **Step 2: Dry-run push**

Run: `git push --dry-run origin main`
Inspect output for any warnings about force or divergence.

- [ ] **Step 3: Push (confirm with user before running)**

Run: `git push origin main`
Expected: pushes all pending commits. No force flag.

- [ ] **Step 4: Create annotated tag**

Run:
```bash
git tag -a v0.1.0 -m "BlastGuard 0.1.0 — Phase 1 MVP

Stdio MCP server with search / apply_change / run_tests tools for
AI coding agents. TypeScript/JavaScript/Python/Rust support via
tree-sitter. BLAKE3 Merkle cache + file watcher. Paired-comparison
benchmark harness (infrastructure complete; full SWE-bench Pro run
gated on upstream SWE-agent fix — see ROADMAP.md).

Measured: 253 Rust lib tests pass, clippy pedantic clean, MCP
handshake + all three tools live-verified against the release binary.
Projected SWE-bench Pro lift: +1 to +3 pp (from adjacent research);
actual number pending upstream unblock."
```

- [ ] **Step 5: Push the tag**

Run: `git push origin v0.1.0`
Expected: tag lands on origin.

- [ ] **Step 6: Verify**

Run: `git ls-remote --tags origin | grep v0.1.0`
Expected: a line showing the tag SHA.

---

## Self-review

**Spec coverage:**
- Framing: Task 1 (README), Task 2 (CHANGELOG), Task 3 (ROADMAP).
- Benchmark honesty: Task 4 (bench/README), Task 5 (KNOWN_GAPS), Task 6 (docs/BENCHMARKING).
- Release discipline: Task 7 (verify), Task 8 (push + tag).

**Placeholder scan:** None. Every task has concrete diffs, commands, and expected output.

**Type consistency:** Not applicable — documentation and git operations only, no new APIs introduced.

**One risk the plan accepts:** Task 8 Step 3 (`git push origin main`) pushes 129+ commits. The user should eyeball the commit list first; a subagent should pause and confirm before running the push. Noted in Step 3.

---

## Execution

Per project memory ("Subagent-Driven → always"), dispatch fresh subagents per task via `superpowers:subagent-driven-development` — no mode prompt. Task 8 Step 3 is the one step that should pause for explicit user confirmation before running.
