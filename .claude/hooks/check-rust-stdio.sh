#!/usr/bin/env bash
# PreToolUse hook — block writes of println!/eprintln!/print!/dbg! into
# committed Rust source. BlastGuard's MCP stdio transport owns stdout, so
# any stdout write corrupts the JSON-RPC protocol frames. Tracing to stderr
# is the only correct way to emit diagnostics (CLAUDE.md hard rule).
#
# Fires on Edit|Write. Reads tool payload from stdin as JSON.
# Exit 2 = block (message on stderr surfaces to Claude).

set -euo pipefail

payload="$(cat)"
file_path="$(printf '%s' "$payload" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null || true)"
[[ -z "${file_path}" ]] && exit 0

# Only police Rust files.
case "${file_path}" in
  *.rs) : ;;
  *) exit 0 ;;
esac

# Skip build scripts, the bench harness, and anything under target/.
case "${file_path}" in
  */target/*|*/build.rs|*/bench/*) exit 0 ;;
esac

# Check the tool's new content, not the file on disk — the hook fires
# BEFORE the write lands. Edit uses `new_string`; Write uses `content`.
new_content="$(printf '%s' "$payload" | jq -r '.tool_input.new_string // .tool_input.content // empty' 2>/dev/null || true)"
[[ -z "${new_content}" ]] && exit 0

# tests modules (#[cfg(test)]) are OK — println!/dbg! are fine in dev.
# We only block when a raw stdout macro appears outside a test scope.
# Cheap heuristic: allow if the offending line's chunk includes "#[cfg(test)]"
# or "mod tests" within the same new_string payload. Not perfect, but
# catches the common case without false-positives on real test code.
if printf '%s' "$new_content" | grep -qE '(#\[cfg\(test\)\]|mod tests)'; then
  exit 0
fi

# Scan for disallowed macros as whole-word invocations.
if printf '%s' "$new_content" | grep -nE '\b(println|eprintln|print|dbg)!' >/dev/null 2>&1; then
  offending="$(printf '%s' "$new_content" | grep -nE '\b(println|eprintln|print|dbg)!' | head -5)"
  >&2 echo "Blocked: disallowed stdout macro in ${file_path}"
  >&2 echo "${offending}"
  >&2 echo "BlastGuard's MCP stdio transport owns stdout — use tracing::{debug,info,warn,error}! instead."
  >&2 echo "If this is a test file, wrap the code under #[cfg(test)] and try again."
  exit 2
fi

exit 0
