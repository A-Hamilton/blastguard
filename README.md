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
# Binary lands at ./target/release/blastguard (≈ 8 MB, stripped + LTO).
```

Requires Rust 1.82+.

## Use with Claude Code

```bash
claude mcp add blastguard -- /absolute/path/to/target/release/blastguard /absolute/path/to/your/project
```

A recommended line for the project's `CLAUDE.md`:

> For multi-file changes where seeing blast radius matters, use BlastGuard's
> `apply_change`. For trivial single-line fixes, your native edit tool is fine.
> Use `search`'s structural patterns (`callers of X`, `outline of FILE`) before
> falling back to grep.

## Features

- **4 languages** — TypeScript, JavaScript, Python, Rust. Go is deferred to post-MVP
  pending evidence that Go tasks are a measurable loss.
- **Sub-500ms warm start** on a 10K-file project via BLAKE3 Merkle cache.
- **Live reindex** — `notify-debouncer-mini` at 100ms keeps the graph current on
  file saves outside the `apply_change` path.
- **Gitignore-aware** — the walker, grep fallback, and watcher all respect
  `.gitignore`.

## Honest positioning

This is a Phase 1 MVP. **Measured SWE-bench Pro lift has not been published yet.**
The harness lives in `bench/`; see `bench/README.md` for the run command.

Projected lift is `+1 to +3 points` on SWE-bench Pro with a realistic confidence
interval of `-1 to +5` — grounded in peer-reviewed adjacent research:

- cAST (arXiv:2506.15655): up to +2.7 Pass@1 on SWE-bench Lite/Verified.
- WarpGrep v2 (Morph): +2.1–3.7 on SWE-bench Pro.
- Auggie (Augment Code): +5.9 over the SWE-Agent Scale-AI scaffold.
- CodeCompass (arXiv:2602.20048): +20pp on hidden-dependency tasks, 0pp on
  semantic tasks — exactly the split BlastGuard's graph-first design predicts.

BlastGuard's strongest durable value is on weaker / cheaper models (Sonnet 4.6,
Haiku 4.5, GLM-5.1) where token efficiency translates directly to cost savings.
Opus 4.7 and Claude Mythos already handle much of what BlastGuard provides
natively; the lift there may be smaller.

Benchmark integrity matters. The grader in `bench/grader.py` defends against the
UC Berkeley "BenchJack" `conftest.py` exploit (any change to `conftest.py`,
`pytest.ini`, `tox.ini`, or CI workflows counts as unresolved tampering).

## Known limitations (Phase 1)

- Cross-file call edges aren't resolved yet — cascade warnings surface callers
  in the same file as the edited symbol only. Resolving across files is a
  Phase 2 item contingent on benchmark data.
- Dynamic dispatch (`getattr`, `obj[method]()`) gets `Confidence::Inferred` and
  surfaces to the agent with a caveat rather than being dropped.
- No Go support.
- Semantic search (`around X`, embeddings) is a feature-flagged Phase 2 item.

## Comparison

| Tool | Graph retrieval | Cascade warnings | Test attribution | Open source | Stack |
|---|---|---|---|---|---|
| BlastGuard (this) | ✅ | ✅ | ✅ | ✅ (MIT) | Rust, rmcp 1.5 |
| [WarpGrep v2](https://morphllm.com/blog/warpgrep-v2) | Retrieval only | — | — | — | closed |
| [Auggie](https://www.augmentcode.com/blog/auggie-tops-swe-bench-pro) | Semantic only | — | — | — | closed |
| [code-graph-mcp](https://github.com/sdsrss/code-graph-mcp) | ✅ | — | — | ✅ | Rust |
| [Replay.io MCP](https://www.replay.io/) | — | — | Debug-focused | — | closed |

## Documentation

- `SPEC.md` — full technical specification (~18 sections).
- `CLAUDE.md` — contributor conventions and build-order discipline.
- `docs/superpowers/plans/` — the 7 implementation plans that produced this
  codebase. Read these if you want to understand why the code is structured
  the way it is.
- `bench/README.md` — benchmark run commands + cost + methodology.

## License

MIT — see `LICENSE`.
