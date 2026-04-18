---
name: blastguard
description: Structural code navigation, cascade-aware edits, and test-failure attribution via the BlastGuard MCP server. Auto-invoke when the user asks about callers of a function, importers of a module, implementors of a trait/interface, inheritance, blast radius of an edit, or wants to run tests after changes and see which of their edits caused failures. Trigger phrases — "who calls", "callers of", "what imports", "what extends", "what implements", "find all uses of", "cascade impact", "run tests and attribute", "what broke".
---

# BlastGuard Tool Routing

This skill routes structural code queries and cascade-aware edits to the
BlastGuard MCP server's three tools. It does NOT replace native tools — it
augments them for cases where BlastGuard's graph-aware response is cheaper
and richer.

## When to use each BlastGuard tool

### `blastguard__search` — AST graph + regex grep

Use **instead of** native Grep or `bash rg/grep` when the query is structural:

| User intent | Query to pass |
|---|---|
| Who calls this function? | `callers of processRequest` |
| What does this function call? | `callees of handler` |
| List all symbols in a file | `outline of src/handler.ts` |
| Find a function by name (exact + fuzzy) | `find processRequest` |
| Which tests exercise this file? | `tests for src/handler.ts` |
| What imports this module? | `importers of src/utils.ts` |
| What does this module import? | `imports of src/api.ts` |
| Path between two symbols | `chain from api to processRequest` |
| External libraries in use | `libraries` |
| Free-text fallback | Any other query — routes to grep, cap 30 hits |

Response: file:line with the function's full inline signature — so a
follow-up `Read` is rarely needed.

### `blastguard__apply_change` — edit with cascade warnings

Use **instead of** native Edit/Write when the change could affect callers
or implementors:

- Signature changes (params, return type)
- Sync ↔ async flips
- Symbol renames or removals
- Trait/interface method additions or signature changes

Response includes up to four cascade warnings (`SIGNATURE`, `ASYNC_CHANGE`,
`ORPHAN`, `INTERFACE_BREAK`) naming the specific callers that may break,
plus a bundled context listing up to 10 callers with inline signatures and
the test files importing the edited file. One apply_change call typically
replaces 4-8 follow-up Grep+Read calls.

For **trivial one-line fixes** (typos, constant updates, comment tweaks),
the native Edit tool is fine — apply_change is overhead you don't need
when blast radius is obviously zero.

### `blastguard__run_tests` — test runner + failure attribution

Use **instead of** `bash pytest/jest/cargo test` when you've made source
edits this session. Auto-detects jest / vitest / pytest / cargo test and
parses the output. The unique value: each failure message is annotated
with `YOU MODIFIED X in file:line (N edits ago)` when a stack-trace
frame lands inside a symbol you recently edited via apply_change. This
closes the feedback loop — the agent reading a failure sees immediately
which of its own changes caused it.

## When NOT to use BlastGuard

- Plain text search with no code-structure intent (log messages, string
  literals, comments) — native Grep is fine.
- Reading one specific known file — native Read is fine.
- Trivial one-line edits with clearly zero blast radius — native Edit is fine.
- File creation in a directory BlastGuard doesn't index (docs, configs) —
  native Write is fine.

## Resource: `blastguard://status`

BlastGuard also exposes a status resource. Read it via the MCP resource
protocol for a one-block project overview: index size, language breakdown,
test runner detected, last test run, recent edits. Useful for grounding
the agent at the start of a long task.

## Reliability note

Per CodeCompass (arXiv:2602.20048): agents don't always reach for novel
tools even when instructed. If BlastGuard's structural search would help
but the agent is defaulting to Grep, restating the query with an explicit
"callers of X" / "imports of Y" phrasing usually triggers the correct
routing.
