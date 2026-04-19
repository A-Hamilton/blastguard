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
