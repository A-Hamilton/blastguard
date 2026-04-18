# Contributing to BlastGuard

Thanks for wanting to contribute. BlastGuard is MIT-licensed and built in
the open. The bar for a good PR is: it passes CI, it does one thing, and it
adds a test that would have caught the bug it fixes (if it's a fix) or pins
the behaviour it adds (if it's a feature).

## Getting set up

You need Rust 1.82+ and (optionally) `uv` 0.11+ if you're touching the
Python benchmark harness.

```bash
git clone https://github.com/A-Hamilton/blastguard.git
cd blastguard
cargo build --release
cargo test
```

The test suite runs in under a second and should finish with `251 passed`.
The binary lands at `./target/release/blastguard`.

If you're touching `bench/`:

```bash
cd bench
uv sync
uv run pytest tests/
```

## The quality gates (what CI enforces)

`.github/workflows/ci.yml` runs these on every PR. Run them locally before
submitting:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -W clippy::pedantic -D warnings
cargo test --all-targets
cargo build --release
```

`-D warnings` turns every clippy pedantic warning into an error. If a lint
feels wrong, don't `#[allow]` it silently — open an issue explaining why,
or add a short comment at the suppression site naming the rule and the
reason.

The Python bench has its own gates:

```bash
cd bench
uv run ruff check .
uv run pytest tests/
```

## What the codebase is trying to do

Read `SPEC.md` first — it's the source of truth for design decisions. Skim
`docs/superpowers/plans/` for the seven implementation plans that produced
every piece of code in `src/`. Plans document *why* specific structures
exist, which saves you time guessing.

The short version:

- `src/graph/` — code graph (symbols, edges, cascade impact).
- `src/parse/` — tree-sitter drivers per language.
- `src/index/` — parallel indexer + BLAKE3 Merkle cache + file watcher.
- `src/search/`, `src/edit/`, `src/runner/` — the three MCP tool backends.
- `src/mcp/` — the rmcp 1.5 stdio server wiring.

## Kinds of contribution that are welcome

- **Bug reports with a minimal repro.** The `bug_report.md` issue template
  walks through the shape.
- **Bug fixes with a regression test.** The fix goes in `src/`, the test
  goes next to it in a `#[cfg(test)] mod tests` block. If the bug crosses
  modules, an integration test in `tests/` is fine.
- **Language driver improvements.** TypeScript / JavaScript / Python / Rust
  drivers live in `src/parse/`. Open an issue first if you want to add a
  fifth language — we deferred Go until benchmark data shows it's worth it
  (`SPEC.md` §Decision Log).
- **Cascade detector improvements.** See `src/graph/impact.rs`. Phase 2
  candidates are listed in `SPEC.md` §5.3.
- **Benchmark runs.** If you run the SWE-bench Pro harness against a model
  (or with different scaffolding), open a PR adding the raw JSONL to
  `bench/results/` and a short note in the README. Honest negative results
  are just as welcome as positive ones — `SPEC.md` §15.3.
- **Claude Code integration tweaks.** Hooks, skill copy, tool description
  adjustments. See `integrations/claude-code/`.

## Kinds of contribution we're pushing back on (for now)

- **Speculative Phase 2 features before benchmark data exists.** `CLAUDE.md`
  spells this out: Phase 2 is gated on Phase 1 numbers. The SPEC's
  "Decision Log" section describes the evidence-first discipline.
- **New dependencies** without a line in the PR describing why the existing
  crate set doesn't cover the use case.
- **Benchmark hacking.** The grader in `bench/grader.py` detects the
  Berkeley BenchJack `conftest.py` exploit; don't work around it.

## Workflow

Fork, branch, open a PR to `main`. Small PRs merge faster than big ones.
If your change touches more than ~3 files, please start with an issue
describing the design — it's cheaper to agree on shape before you code.

Commits: imperative mood, subject ≤ 72 chars, body paragraph when the *why*
needs explaining. We don't require any particular commit convention beyond
readability.

## Rust quality rules (the short list)

- No `println!` / `eprintln!` — `tracing::{info,warn,error}!` to stderr.
  The MCP stdio protocol owns stdout.
- No `.unwrap()` / `.expect()` in production paths. Exceptions: `#[cfg(test)]`,
  or truly unreachable branches with a comment naming the invariant.
- No `panic!` / `todo!` / `unimplemented!` in committed code.
- `#[must_use]` on `Result`-returning constructors.
- `///` docs on every `pub` item; `# Errors` section on fallible public fns.

## Getting help

- Open an issue on GitHub for anything.
- Read `SPEC.md` + `CLAUDE.md` + the plans in `docs/superpowers/plans/`
  before asking — most design questions are answered there.

## Code of conduct

See `CODE_OF_CONDUCT.md`. Be kind, be specific, assume good faith.
