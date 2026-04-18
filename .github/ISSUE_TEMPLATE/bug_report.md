---
name: Bug report
about: Something doesn't work the way the docs say it should
labels: bug
---

## What happened

<!-- One or two sentences. What did BlastGuard do that it shouldn't have? -->

## What you expected

<!-- One sentence. What should have happened instead? -->

## Minimal reproduction

<!-- A small project (or a link to one), the BlastGuard command / MCP call
you ran, and the output you got. Hard bugs get fixed fast when they come
with a clean repro; vague reports can sit for weeks. -->

```bash
# example
cargo run --release -- /path/to/test-project
# then in your MCP client:
# tool: search
# query: "callers of someFunction"
# got: ...
# expected: ...
```

## Environment

- BlastGuard version (commit SHA or release tag):
- Rust version (`rustc --version`):
- OS + kernel (`uname -a` on Linux/macOS):
- MCP client (Claude Code / other):

## Logs

<!-- If BlastGuard's stderr says anything interesting, paste it here.
Run with `BLASTGUARD_LOG=debug` for more detail. -->

```
```
