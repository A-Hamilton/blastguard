#!/usr/bin/env bash
# PreToolUse hook for native Grep. Reinforces BlastGuard's `search` when
# the grep pattern looks structural (function/class/import markers). On
# plain text patterns (strings, comments, log messages) the hook is silent.
#
# Install: wire into .claude/settings.json per integrations/claude-code/
# settings.sample.json. Requires `jq`.
#
# Input shape (stdin JSON):
#   {
#     "tool_name": "Grep",
#     "tool_input": { "pattern": "...", "path": "...", ... },
#     ...
#   }
#
# Output on stdout — a single JSON object. Always permissionDecision:"allow"
# so the user's Grep call still runs; the additionalContext field nudges
# the model toward blastguard__search on the NEXT turn.

set -eu

if ! command -v jq >/dev/null 2>&1; then
    # Fail open: emit no output so Claude Code treats the hook as a no-op.
    exit 0
fi

HOOK_INPUT=$(cat)
PATTERN=$(jq -r '.tool_input.pattern // ""' <<<"$HOOK_INPUT")

# Structural intent regex. Keep narrow — false positives train the model
# to ignore the reminder.
STRUCTURAL='(^|[[:space:]])(fn |impl |trait |class |interface |enum |struct |def |async def |function |extends |implements |import |from |use |require\()'

if [[ "$PATTERN" =~ $STRUCTURAL ]]; then
    jq -n '
    {
        hookSpecificOutput: {
            hookEventName: "PreToolUse",
            permissionDecision: "allow",
            additionalContext: "Structural pattern detected in Grep. Consider blastguard__search next time — it returns inline signatures + callers + tests in one response. Structural queries: `callers of X`, `outline of FILE`, `tests for FILE`, `imports of FILE`, `find X`."
        }
    }'
fi
