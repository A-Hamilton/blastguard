# BlastGuard

Open-source Rust MCP server that lifts AI coding agents via AST-graph retrieval,
cascade warnings on edits, and test-failure attribution. Single binary, stdio
transport, MIT licensed.

BlastGuard exposes three tools over the Model Context Protocol:

- **`search`** — AST-graph queries (`callers of X`, `tests for FILE`, `outline of FILE`, …)
  with inline signatures, plus regex grep fallback. Typical result: 50–300 tokens.
- **`apply_change`** — edit files with cascade warnings (SIGNATURE / ASYNC_CHANGE /
  ORPHAN / INTERFACE_BREAK) and a bundled context (callers + tests). Writes immediately;
  no approval gate.
- **`run_tests`** — auto-detects jest / vitest / pytest / cargo, annotates failures with
  `YOU MODIFIED X (N edits ago)` so the agent links test breakage to its own recent edits.

It also serves `blastguard://status` as a resource — a compact one-block project overview.

## Install

```bash
git clone https://github.com/A-Hamilton/blastguard.git
cd blastguard
cargo build --release
# Binary lands at ./target/release/blastguard (approximately 8-9 MB, stripped + LTO).
```

Requires Rust 1.82+.

## Use with Claude Code

### 1. Register the MCP server

```bash
claude mcp add blastguard --scope project -- \
    /absolute/path/to/target/release/blastguard \
    /absolute/path/to/your/project
```

### 2. Enable the full routing integration (recommended)

`integrations/claude-code/` ships a drop-in skill + three PreToolUse hooks
that reinforce BlastGuard's tools during every session. See
`integrations/claude-code/README.md` for install steps.

- **Skill** auto-activates on trigger phrases ("callers of", "what imports",
  "find X") and survives context compaction.
- **Hooks** fire on every matching native tool call:
  - `Grep` with a structural pattern (`fn `, `impl `, `class `, `import`) →
    reminds the agent `blastguard__search` returns richer context.
  - `Bash` with `rg`/`grep`/`ack` or a test-runner invocation → nudges toward
    the relevant BlastGuard tool.
  - `Edit` / `Write` on a source file → suggests `blastguard__apply_change`
    for cascade awareness.

All hooks `permissionDecision: "allow"` — they never block the user's chosen
tool, only inject `additionalContext` for the next turn.

### Honest positioning of the routing layer

Per [CodeCompass (arXiv:2602.20048)](https://arxiv.org/abs/2602.20048), forced
tool use actively hurts performance — agents ignore gated tools ~58% of the
time and stall on error paths. BlastGuard's integration biases rather than
forces. Expect ~40–60% of routine queries to still reach for native tools
unless the user's phrasing makes structural intent obvious. The durable value
is strongest on hard tasks: multi-file refactors, cascade analysis, test
attribution after a batch of edits.

## Features

- **4 languages + JSX** — TypeScript, JavaScript, Python, Rust.
  TSX/JSX parsed with the correct tree-sitter grammar; `<Component />`
  and `<Radix.Button>` JSX usages become first-class Calls edges.
  Modern idioms covered: arrow-function consts (`const Foo = () =>
  {}`), async arrows, class methods with full signatures, Python
  package-relative imports (`.sub.leaf`, `..mid`), CommonJS
  `require()`, Rust `pub use sibling::X` and `mod_item` declarations,
  TypeScript `tsconfig.json` path aliases (`@shared/*`). Go is
  deferred to post-MVP pending evidence that Go tasks are a
  measurable loss.
- **Cross-file cascade warnings** — edit a function in one file and
  `apply_change` names every caller across the project that would
  break. Resolver chain runs after every re-index path (cold_index,
  warm_start, watcher, apply_change reparse), so the warnings stay
  accurate through live edits.
- **Sub-500ms warm start** on a 10K-file project via BLAKE3 Merkle
  cache.
- **Live reindex** — `notify-debouncer-mini` at 100ms keeps the graph
  current on file saves outside the `apply_change` path.
- **Gitignore-aware** — the walker, grep fallback, and watcher all
  respect `.gitignore`.
- **Compact path rendering** — every agent-facing response (tool
  hits, warning bodies, bundled context, test attribution) renders
  project-relative paths, never absolute tempdir / home prefixes.

## Honest positioning

This is a Phase 1 MVP with a specific, measured value proposition — and a
specific claim that is **not yet verified**.

### What the measurements actually show (Round 13, Gemma 4 26B A4B, full 10-task × 3-seed × 2-arm suite)

- **3–10× cheaper.** Median input tokens −61% to −87% on 7 of 10 tasks; BG
  more expensive on 3 multi-hop tasks where it calls BlastGuard + native
  tools together.
- **Faster on every task.** Median wall delta ranges from −13% (trace-cache)
  to −88% (callers-apply-edit). Only chain-search is ≈tied (+2%).
- **Tied on correctness.** Deterministic substring grader: BG 22/30 vs raw
  23/30. Judge correctness axis: BG 5 / raw 10 / ties 15. Not a correctness
  win, not a correctness loss — statistically a tie.
- **Loses on substance.** LLM-judge substance axis: raw 20 wins, BG 7. BG's
  palette-constrained answers are shorter and thinner than raw's free-form
  exploration. BG wins conciseness 8–7–15, which is the same phenomenon from
  the other side.

**Honest framing: a cost-quality tradeoff.** BlastGuard is decisively cheaper
and faster at roughly-equivalent correctness on this model. It is not
currently a quality-ahead tool by absolute measure.

See `docs/MICROBENCH.md` Round 13 section for the full tables and prior
rounds' learning trajectory.

### What is NOT measured (and why)

**Downstream-task lift on SWE-bench Pro / Verified** — zero.

The Phase 1 measurement is micro-bench questions on this codebase, not
hidden-dependency bug fixes on downstream repos. The bench harness (`bench/`)
is built for SWE-bench, but a real Pro run is blocked by an upstream SWE-agent
Docker tag-length bug (see `bench/KNOWN_GAPS.md` Gap 5). The unblock is to
pivot to SWE-bench Verified, which uses shorter image tags; this is planned
work, not done work.

Until that run lands, **do not quote a +1-3pp SWE-bench lift number from this
README.** That claim is a prior-based projection, not a measurement.

### Where adjacent research suggests BG *should* help

- **CodeCompass (arXiv:2602.20048):** +20pp on hidden-dependency tasks, 0pp
  on semantic tasks. Round 13 dilutes this across a mixed task set; a
  hidden-dependency-weighted benchmark should concentrate the effect.
- **cAST (arXiv:2506.15655):** +2.67 Pass@1 on SWE-bench Lite.
- **WarpGrep v2 (Morph):** +2.1–3.7 on SWE-bench Pro.
- **Auggie (Augment Code):** +5.9 over SWE-Agent Scale-AI scaffold.

These are adjacent, not equivalent — they test different tools under different
conditions. Our own measurement will supersede them once it exists.

### Model sensitivity (unmeasured on stronger reasoners)

All quality measurements above are on Gemma 4 26B A4B (Q4_K_M, local,
thinking-mode). Cloud models (Sonnet 4.6, Opus 4.7) may close the substance
gap — their richer reasoning might use BG's palette more fully. This is
unmeasured. A partial Qwen 3.6 35B A3B run was attempted but exposed model-
specific quirks (DONE-emission reliability, tool-call template handling)
that require further infrastructure work before a fair comparison.

### Benchmark integrity

The grader in `bench/grader.py` defends against the UC Berkeley "BenchJack"
`conftest.py` exploit — any change to `conftest.py`, `pytest.ini`, `tox.ini`,
or CI workflows counts as unresolved tampering. Unrelated to Phase 1's lift
claim; included because tampering-aware grading is a small-but-real
differentiator for any SWE-bench-adjacent tool.

### What's verified today (not projected)

- Rust codebase: 284 library tests pass, clippy pedantic clean,
  `cargo fmt` clean.
- MCP handshake + all three tools live-tested against the release
  binary. Every MCP-facing response renders project-relative paths —
  no absolute tempdir / home prefixes leak into agent context.
- **Cross-file resolution works end-to-end** for Rust, Python, TS,
  JS, and TSX. Verified via a live-probe harness that seeds tempdir
  fixtures and queries BlastGuard over stdio:
  - Rust: `use crate::foo` / `use sibling_mod::X` / `pub use mod::Y`
    all resolve. `mod_item` declarations appear in outlines.
  - TS/TSX: relative imports, `tsconfig.json` path aliases
    (`@shared/*`), JSX component calls (`<Button>`, `<Radix.Button>`),
    arrow-const declarations (`const Foo = () => {}`).
  - JS: ES-module `import` AND CommonJS `require('./x')`, arrow-const
    declarations, full method signatures in outlines.
  - Python: absolute dotted imports AND package-relative (`.sub.leaf`,
    `..mid`) imports, class-method callers, cross-file cascade
    warnings on `apply_change`.
- **Cascade warnings** fire cross-file on `apply_change`: edit a
  function in one file, the SIGNATURE warning names the callers
  in other files that would break.
- **Quality measurement framework** (priority-ordered, per user spec):
  - Priority 1a: `bench/microbench_grader.py` — deterministic
    substring grading; BG correctness must stay within 2pp of raw.
  - Priority 1b: `bench/microbench_judge.py` — LLM-as-judge with
    blind randomized A/B pairwise ranking across three axes
    (correctness, substance, conciseness), opt-in via
    `microbench --run-judge`.
  - Priority 2: `bench/stats_aggregate.py` — input/output token
    deltas with paired-difference 95% CI.
  - Priority 3: wall time — indicator only on local Gemma
    (thinking-mode inflates wall beyond cloud-API reality).
- **Measured micro-benchmark results on Gemma-4 26B A4B (Round 13,
  full 10-task × 3-seed × 2-arm, local, zero API cost):** a clean
  cost-quality tradeoff. See `docs/MICROBENCH.md` Round 13 for the
  full tables; summary in "Honest positioning" above. Prior rounds
  document the learning trajectory (including a Round-12 result that
  Round 13 overturned at n=3 — a good example of why single-seed
  findings should be treated as hypotheses).

What's pending: real downstream-task lift on SWE-bench Verified.
Waiting on the pivot from SWE-bench Pro (blocked by Gap 5) to
Verified (shorter image tags, unblocked path).

## Known limitations (Phase 1)

- Cross-file call resolution is **unambiguous-only** — when exactly
  one file that a caller imports declares a symbol with the called
  name, the Calls edge is rewritten to that file and the edge's
  confidence becomes `Inferred`. When two or more imports declare
  the same name, the edge stays `Unresolved` and `callers of X`
  falls back to a per-importer-file hint so the agent can grep them.
- **Re-export chains don't follow through.** A `pub use
  inner::fn_name` in `mod.rs` doesn't teach the resolver that
  `fn_name` is reachable through that module. `chain from X to Y`
  falls back to listing both endpoints plus a "bridge via imports
  of / callers of" hint when a path can't be found. Full re-export
  resolution is Phase 2.
- Dynamic dispatch (`getattr`, `obj[method]()`) gets
  `Confidence::Inferred` and surfaces to the agent with a caveat
  rather than being dropped.
- **CommonJS named-export assignments** (`module.exports.name =
  () => ...`) are not captured as symbols. Rare in modern code;
  ES-module `export` and `module.exports = { ... }` shorthand both
  work.
- No Go support.
- Semantic search (`around X`, embeddings) is a feature-flagged
  Phase 2 item.

## Related work

BlastGuard sits in an ecosystem of tools exploring how to make AI coding
agents more effective. Each of these tackles a different slice of the
problem; none is a competitor so much as a neighbour.

| Project | Focus | Open source |
|---|---|---|
| BlastGuard (this) | Graph retrieval + cascade warnings + test attribution | MIT |
| [code-graph-mcp](https://github.com/sdsrss/code-graph-mcp) | Open-source AST graph MCP | MIT |
| [WarpGrep v2](https://morphllm.com/blog/warpgrep-v2) | RL-trained search subagent | closed |
| [Auggie](https://www.augmentcode.com/blog/auggie-tops-swe-bench-pro) | Semantic retrieval / context engine | closed |
| [Replay.io MCP](https://www.replay.io/) | Runtime-debugging MCP | closed |

If you're working on an adjacent project and want to cross-link, open a PR
against this table.

## Documentation

- `SPEC.md` — full technical specification (~18 sections).
- `CLAUDE.md` — contributor conventions and build-order discipline.
- `docs/superpowers/plans/` — the 7 implementation plans that produced this
  codebase. Read these if you want to understand why the code is structured
  the way it is.
- `bench/README.md` — benchmark run commands + cost + methodology.

## License

MIT — see `LICENSE`.
