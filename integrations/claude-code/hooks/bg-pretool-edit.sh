#!/usr/bin/env bash
# PreToolUse hook for native Edit / Write. Nudges toward blastguard__apply_change
# when the target file is source code (.ts/.tsx/.js/.jsx/.py/.rs) — apply_change
# returns cascade warnings + caller/test context that Edit can't produce.
#
# Silent when editing docs, configs, or non-source files (apply_change wouldn't
# add value there — no graph to analyse).
#
# Requires jq.

set -eu

if ! command -v jq >/dev/null 2>&1; then
    exit 0
fi

HOOK_INPUT=$(cat)
FILE_PATH=$(jq -r '.tool_input.file_path // .tool_input.path // ""' <<<"$HOOK_INPUT")

# Source extensions BlastGuard parses via tree-sitter.
SOURCE_EXT='\.(ts|tsx|mts|cts|js|jsx|mjs|cjs|py|pyi|rs)$'

if [[ "$FILE_PATH" =~ $SOURCE_EXT ]]; then
    jq -n '
    {
        hookSpecificOutput: {
            hookEventName: "PreToolUse",
            permissionDecision: "allow",
            additionalContext: "Source-file edit detected. For changes that could affect callers (signature changes, symbol rename, async flip, interface/trait update, symbol removal), prefer blastguard__apply_change — it returns cascade warnings (SIGNATURE / ASYNC_CHANGE / ORPHAN / INTERFACE_BREAK) and a bundled caller+test context in one response. Native Edit is fine for trivial one-line fixes where blast radius is clearly zero."
        }
    }'
fi
