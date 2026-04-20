#!/usr/bin/env bash
# PostToolUse hook — when a tree-sitter query file under `queries/` is
# edited, run just the tests for that language's parser. Closes the
# "did I break the query?" feedback loop from ~30s (full `cargo test`)
# down to ~2s. This session hit three query bugs that each cost a
# minute of debugging because they only surfaced on the next full
# test run:
#   - JSX captures against plain TS grammar (NodeType error)
#   - `(function)` vs `(function_expression)` node-type name
#   - predicate `(#match? ...)` placed outside the capture
#
# Fires on Edit|Write. Reads tool payload from stdin as JSON.
# Exit 2 = block (message on stderr surfaces to Claude).
# Exit 0 = allow silently (non-.scm files, passing tests, or skipped).

set -euo pipefail

payload="$(cat)"
file_path="$(printf '%s' "$payload" | jq -r '.tool_input.file_path // .tool_input.path // empty' 2>/dev/null || true)"
[[ -z "${file_path}" ]] && exit 0
[[ ! -f "${file_path}" ]] && exit 0

# Only police the repo's tree-sitter query files.
case "${file_path}" in
  */queries/typescript.scm) lang="typescript" ;;
  */queries/tsx.scm)        lang="typescript" ;;   # TSX tests live in parse::typescript
  */queries/javascript.scm) lang="javascript" ;;
  */queries/python.scm)     lang="python" ;;
  */queries/rust.scm)       lang="rust" ;;
  *) exit 0 ;;
esac

# Find the project root (directory containing Cargo.toml). If we can't
# figure it out, exit silently rather than run from the wrong cwd.
dir="$(dirname "${file_path}")"
project_root=""
while [[ "${dir}" != "/" && "${dir}" != "" ]]; do
  if [[ -f "${dir}/Cargo.toml" ]]; then
    project_root="${dir}"
    break
  fi
  dir="$(dirname "${dir}")"
done
[[ -z "${project_root}" ]] && exit 0

# Run just the parser-module tests for the affected language. Quiet
# mode so a clean pass is silent; failures dump to stderr for Claude.
cd "${project_root}"
output="$(cargo test --lib "parse::${lang}::tests" -- --quiet 2>&1 || true)"
if printf '%s' "${output}" | grep -qE 'test result: ok|running 0 tests'; then
  exit 0
fi

# Something failed — surface enough context for Claude to react.
>&2 echo "check-scm-breakage: parse::${lang}::tests failed after edit to ${file_path##*/}"
>&2 printf '%s\n' "${output}" | tail -40
exit 2
