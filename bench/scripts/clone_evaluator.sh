#!/usr/bin/env bash
# bench/scripts/clone_evaluator.sh
set -euo pipefail

TARGET="${SCRIPT_DIR:-$(dirname "$0")/..}/.evaluator"
if [ -d "$TARGET/.git" ]; then
  echo "evaluator already cloned at $TARGET"
  exit 0
fi
git clone --depth 1 https://github.com/scaleapi/SWE-bench_Pro-os "$TARGET"
cd "$TARGET"
pip install -r requirements.txt
echo "evaluator ready at $TARGET"
