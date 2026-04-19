#!/usr/bin/env bash
# bench/scripts/verify_sweagent.sh
# Confirms SWE-agent is importable and its CLI responds.
#
# Requires bench/.sweagent-repo to exist (run clone_sweagent.sh first).
# The package on PyPI (sweagent 0.0.1) is a non-functional stub; we install
# from git via [tool.uv.sources] in pyproject.toml and point the three env
# vars at the cloned repo's config/ + tools/ + trajectories/ directories.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BENCH_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$BENCH_DIR/.sweagent-repo"
VENV_PYTHON="$BENCH_DIR/.venv/bin/python"
VENV_SWEAGENT="$BENCH_DIR/.venv/bin/sweagent"

if [[ ! -d "$REPO_DIR" ]]; then
    echo "ERROR: $REPO_DIR not found. Run: bash bench/scripts/clone_sweagent.sh" >&2
    exit 1
fi

export SWE_AGENT_CONFIG_DIR="$REPO_DIR/config"
export SWE_AGENT_TOOLS_DIR="$REPO_DIR/tools"
export SWE_AGENT_TRAJECTORY_DIR="$REPO_DIR/trajectories"

echo "--- sweagent Python import ---"
"$VENV_PYTHON" -c "import sweagent; print('sweagent version:', sweagent.__version__)"

echo ""
echo "--- sweagent --help (first 30 lines) ---"
"$VENV_SWEAGENT" --help 2>&1 | head -30

echo ""
echo "sweagent verification OK"
echo "CLI binary: $VENV_SWEAGENT"
