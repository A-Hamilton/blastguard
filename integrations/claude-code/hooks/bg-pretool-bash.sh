#!/usr/bin/env bash
# PreToolUse hook for native Bash. Catches the common case of agents
# grepping via `bash rg ...` / `bash grep ...` / `find | xargs grep` when
# they should be using blastguard__search. Also nudges on direct test-
# runner invocation (pytest / jest / cargo test) toward run_tests.
#
# Always permissionDecision:"allow" — the Bash call still runs. The
# reminder shifts the agent on the next turn.
#
# Requires jq.

set -eu

if ! command -v jq >/dev/null 2>&1; then
    exit 0
fi

HOOK_INPUT=$(cat)
COMMAND=$(jq -r '.tool_input.command // ""' <<<"$HOOK_INPUT")

# Match grep-family invocations with structural-looking patterns.
GREP_CMD='(^|[[:space:]])(rg|grep|ack|ag)([[:space:]]|$)'
STRUCTURAL_TOKENS='(fn |impl |trait |class |interface |enum |struct |def |async def |function |extends |implements |^import |^from |^use )'

# Match direct test-runner invocation.
TEST_CMD='(^|[[:space:]])(pytest|jest|vitest|cargo[[:space:]]+test|npm[[:space:]]+test|yarn[[:space:]]+test|pnpm[[:space:]]+test)([[:space:]]|$)'

context=""

if [[ "$COMMAND" =~ $GREP_CMD ]] && [[ "$COMMAND" =~ $STRUCTURAL_TOKENS ]]; then
    context="Structural grep detected. blastguard__search with 'callers of X' / 'outline of FILE' / 'imports of FILE' / 'find X' returns inline signatures and related tests in one response — cheaper than grep+follow-up reads."
elif [[ "$COMMAND" =~ $TEST_CMD ]]; then
    context="Direct test-runner invocation detected. blastguard__run_tests auto-detects the runner AND annotates failures with 'YOU MODIFIED X (N edits ago)' linking test breakage to your recent edits — use after apply_change for attribution."
fi

if [[ -n "$context" ]]; then
    jq -n --arg ctx "$context" '
    {
        hookSpecificOutput: {
            hookEventName: "PreToolUse",
            permissionDecision: "allow",
            additionalContext: $ctx
        }
    }'
fi
