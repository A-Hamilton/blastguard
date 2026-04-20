#!/usr/bin/env bash
# PostToolUse hook — surface rustfmt drift on Rust source edits. Non-
# blocking: prints a hint to stderr so Claude sees it and can run
# `cargo fmt` before the next cargo test / clippy cycle, but does not
# fail the edit.
#
# Fires on Edit|Write after the write has landed.

set -euo pipefail

payload="$(cat)"
file_path="$(printf '%s' "$payload" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null || true)"
[[ -z "${file_path}" ]] && exit 0
[[ ! -f "${file_path}" ]] && exit 0

case "${file_path}" in
  *.rs) : ;;
  *) exit 0 ;;
esac

case "${file_path}" in
  */target/*) exit 0 ;;
esac

# rustfmt --check exits non-zero on any drift; we want a passive hint
# rather than a block, so we swallow the exit code and advise.
if ! rustfmt --edition 2021 --check "${file_path}" >/dev/null 2>&1; then
  >&2 echo "rustfmt: ${file_path} has formatting drift. Run \`cargo fmt\` before the next cargo check/test cycle."
fi

exit 0
