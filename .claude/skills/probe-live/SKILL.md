---
name: probe-live
description: Build a minimal tempdir fixture for a named parser/resolver scenario, cold-index it with the release BlastGuard binary, and print the hits from a single MCP search query. User-only — codifies the live-probe loop that surfaced 12 correctness bugs in the last session.
disable-model-invocation: true
---

# Probe Live

User-only. Type `/probe-live <fixture> <query>` to run.

## What this replaces

The probe loop I kept rewriting by hand this session — `mktemp -d`,
write a 2-3 file fixture for whatever pattern I'm testing,
`git init`, `cargo build --release`, shell a JSON-RPC payload through
`target/release/blastguard`, parse the hit list. ~40 lines of bash +
python per probe.

This skill packages all of that behind a named fixture.

## Usage

```bash
/probe-live <fixture_name> "<search query>"
```

Examples:

```bash
/probe-live python-relative "callers of leaf"
/probe-live tsx-arrow-consts "callers of Arrow"
/probe-live tsconfig-alias "imports of src/app.ts"
/probe-live rust-siblings "libraries"
```

## How it works

`.claude/skills/probe-live/probe.py` is a small self-contained
script that:

1. Creates a tempdir.
2. Seeds it from one of the named fixture templates below.
3. Runs `git init -q` in the tempdir (required because BlastGuard's
   walker skips content without a git root outside of `.venv`
   hidden-dir filtering).
4. Cold-indexes the tempdir via `target/release/blastguard` over
   stdio, sending exactly three JSON-RPC frames:
   `initialize`, `initialized` notification, and
   `tools/call name=search arguments={"query":"..."}`.
5. Pretty-prints the hit list (file:line signature) from the
   structured-content response.

On failure (e.g. binary missing, fixture name typo, MCP error), it
surfaces the error verbatim so the caller can debug.

## Fixture presets

| name | seeds | what it's good for |
|---|---|---|
| `python-relative` | `src/pkg/{mid.py, sub/leaf.py, sub/deep.py, __init__.py, sub/__init__.py}` with `from .sub.leaf import leaf` / `from ..mid import mid` | `resolve_py` relative-dot handling |
| `python-absolute` | `src/utils/auth.py` + `src/handler.py` with `from utils.auth import login` | Python absolute dotted imports |
| `tsx-arrow-consts` | `src/Arrow.tsx` (arrow-const + default export), `src/App.tsx` (`<Arrow />` JSX call) | TSX grammar, arrow-const symbol extraction, JSX-as-call |
| `tsconfig-alias` | `tsconfig.json` with `"@shared/*": ["src/shared/*"]`, `src/shared/greet.ts`, `src/app.ts` importing `@shared/greet` | tsconfig alias resolution + LibraryImport pruning |
| `rust-siblings` | `src/graph/{mod.rs, impact.rs}`, `src/lib.rs` with `pub use impact::X;` | Sibling-module fs-resolution in the Rust parser |
| `ts-relative` | `src/handler.ts` importing `./utils/auth` | Bread-and-butter relative TS import (regression canary) |

Adding a new fixture: append a `FIXTURE_<NAME>` dict to `probe.py`'s
fixture registry. Keys are relative paths, values are file contents.

## When NOT to use this

- Performance measurement — use `bench-rerun` instead.
- Production benchmarks — this is for correctness probes only.
- Probing against the live BlastGuard repo itself — just hit
  `target/release/blastguard /home/adam/Documents/blastguard`
  directly; no fixture needed.

## Maintenance

The fixtures are deliberately tiny (~2-5 files each) so the probe
runs in under a second on a cold release binary. If a real-world
edge case starts to matter, add a targeted fixture rather than
growing an existing one — small, focused fixtures make it obvious
which pattern is broken when a probe fails.
