# SWE-agent Scaffold Integration (Plan 9) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace our custom agent loop with [SWE-agent](https://github.com/SWE-agent/SWE-agent) as the scaffold, and integrate BlastGuard's three MCP tools as a SWE-agent "bundle" so arm A (raw SWE-agent) and arm B (SWE-agent + BlastGuard bundle) become apples-to-apples.

**Why:** Plan 8's custom loop was a prompt-exchange shell — it never registered tools, never cloned workspaces, never extracted real patches. We'd have spent $40+ measuring prompt steering instead of tool use. SWE-agent handles workspace management, tool dispatch, patch extraction, Docker isolation, retry — battle-tested on the public leaderboard. Plugging BlastGuard into it via a YAML bundle is ~200 LOC of our code.

**Architecture:** SWE-agent orchestrates per-task: clone repo → checkout base_commit → run agent in Docker → extract git diff as patch. BlastGuard ships as a "bundle" — a directory of shell wrappers + `config.yaml` + a thin Python bridge that spawns the `blastguard` binary as an MCP stdio subprocess and forwards one request per call. Our `bench/runner.py` becomes an orchestrator that invokes SWE-agent twice per task (once without the BlastGuard bundle, once with it) against the same instance_id with the same seed.

**Tech Stack:**
- `sweagent` (PyPI package; active 2026, YAML-bundle tool system, OpenRouter-compatible via `LITELLM_MODEL`/`base_url` overrides)
- Python 3.12, uv for dependency management
- Docker (already required for SWE-bench_Pro-os evaluator)
- BlastGuard's existing Rust binary (no changes needed)
- `mcp` Python SDK for the bridge (client side)

**Scope (MVP):**
- Adopt SWE-agent as the single scaffold for both arms
- Write BlastGuard bundle: `config.yaml` + 3 shell wrappers + `bridge.py`
- Rewrite `bench/runner.py` to orchestrate SWE-agent invocations
- Retire `bench/agent_loop.py` (delete or keep as historical reference only)
- Preserve `bench/budget.py`, `bench/telemetry.py`, `bench/stats.py`, `bench/evaluator.py`, `bench/compare.py` — these are scaffold-agnostic
- Paired 10-task smoke + 100-task pilot still apply; costs re-verified in Task 10

**Out of scope (deferred):**
- Long-lived BlastGuard daemon (one-shot subprocess per tool call — ~200ms overhead is tolerable for the 10-30 calls per task)
- Custom SWE-agent agent/action configs beyond what the default SWE-agent config ships with
- Non-Python tasks (same constraint as Plan 8)

---

## Research notes (verified against SWE-agent v1.1.0, 2026-04-19)

**These notes override any code block in Tasks 3-9 that contradicts them.** The original plan was written against assumed flags; Task 1 verified the real interface.

**Install path:** `sweagent @ git+https://github.com/SWE-agent/SWE-agent.git@v1.1.0` (PyPI `sweagent` is a broken v0.0.1 stub). The repo is also cloned to `bench/.sweagent-repo/` for bundled `config/`, `tools/`, `trajectories/` directories that the package expects at runtime.

**CLI surface:**
- `sweagent run` — one instance. Flags: `--config <yaml>`, `--agent.model.name`, `--agent.model.per_instance_cost_limit`, `--env.repo.path` (local repo path) OR `--env.repo.github_url`, `--problem_statement.path` OR `--problem_statement.github_url`, `--output_dir`. **No `--seed`, `--workspace`, `--tools.bundles`, `--instance.repo` flags exist.** Unknown flags error out (`extra="forbid"`).
- `sweagent run-batch` — multiple instances. Flags: `--instances.type huggingface --instances.dataset_name ScaleAI/SWE-bench_Pro --instances.split test --instances.filter <regex> --instances.slice :N --num_workers N --output_dir <dir>`. Handles repo checkout + Docker internally.

**Tool bundles:** Added via config YAML `agent.tools.bundles: [{path: tools/foo}, ...]`, NOT via CLI flag. Bundle layout: `<bundle>/bin/<executable>` (no `.sh`), `<bundle>/config.yaml` with `tools:` block (not `commands:`). Bundle config schema:
```yaml
tools:
  my_tool:
    signature: "my_tool <arg>"
    docstring: >
      Description...
    arguments:
      - name: arg
        type: string
        required: true
```

**Model config (YAML):**
```yaml
agent:
  model:
    name: "openrouter/minimax/minimax-m2.7"
    api_key: "$OPENROUTER_API_KEY"     # $-prefix means env var lookup
    api_base: "https://openrouter.ai/api/v1"
    per_instance_cost_limit: 3.0
    total_cost_limit: 0.0               # aggregate cap; 0 = disabled
    temperature: 0.0
    max_input_tokens: 200000            # required if model not in litellm.model_cost
    max_output_tokens: 8192
  tools:
    bundles:
      - path: tools/registry            # relative to SWE-agent repo root
      - path: <abs path to blastguard bundle>
    enable_bash_tool: true
    parse_function:
      type: function_calling
```

**Determinism:** There is no seed flag. Use `temperature: 0.0`. Paired arms against the same filter/slice at temp 0 give ~deterministic task ordering; residual non-determinism is inherent to the model.

**Trajectory output schema:** Per instance SWE-agent writes to `<output_dir>/<instance_id>/`:
- `<instance_id>.traj` — JSON with `info.submission` (unified diff string, maybe null), `info.model_stats.instance_cost`, `info.model_stats.tokens_sent`, `info.model_stats.tokens_received`, `info.model_stats.api_calls`, `info.exit_status`. **NOT `prompt_tokens` / `completion_tokens` / `n_turns` / `model_patch`.**
- `<instance_id>.pred` — SWE-bench submission format: `{"instance_id", "model_name_or_path", "model_patch"}`. `model_patch` is null when the agent didn't emit a submission (timeout, cost limit, etc.).

**Parsing telemetry:** read `.traj` for token counts + cost; read `.pred` for the patch. Map to `TokenCount` via:
```python
stats = traj["info"].get("model_stats", {})
tokens = TokenCount(
    input=int(stats.get("tokens_sent", 0)),
    cached_input=0,          # not surfaced by SWE-agent; leave 0
    output=int(stats.get("tokens_received", 0)),
    turns=int(stats.get("api_calls", 0)),
)
patch = pred.get("model_patch") or ""   # coerce null to empty string
```

**Infra failures:** `.traj` `info.exit_status` values include `"submitted"` (clean), `"exit_cost"`, `"exit_error"`, `"exit_format"`, `"exit_context"`. Task 7's `infra_failure` logic should treat any non-`"submitted"` status + empty-patch pred as infra (not task failure) to avoid contaminating McNemar's.

**OpenRouter routing:** `api_key: "$OPENROUTER_API_KEY"` + `api_base: "https://openrouter.ai/api/v1"`. LiteLLM passes through. Export `OPENROUTER_API_KEY` in shell before running.

---

## File Structure

**Create:**
- `bench/bundles/blastguard/config.yaml` — SWE-agent bundle declaring the 3 BG tools
- `bench/bundles/blastguard/bg_search.sh` — shell wrapper calling bridge.py for `search`
- `bench/bundles/blastguard/bg_apply_change.sh` — shell wrapper for `apply_change`
- `bench/bundles/blastguard/bg_run_tests.sh` — shell wrapper for `run_tests`
- `bench/bundles/blastguard/bridge.py` — MCP client bridge: spawns `blastguard`, sends one request, prints response, exits
- `bench/sweagent_runner.py` — orchestrates one SWE-agent invocation per task per arm
- `bench/tests/test_bridge.py` — bridge unit tests with a fake MCP server
- `bench/tests/test_sweagent_runner.py` — runner tests (mocked SWE-agent subprocess)

**Modify:**
- `bench/pyproject.toml` — add `sweagent` and `mcp` dependencies
- `bench/runner.py` — thin wrapper calling `sweagent_runner.run_arm()`
- `bench/README.md` — document new workflow

**Delete / trim:**
- `bench/agent_loop.py` — remove `run_anthropic`, `_run_openai_compatible_async`, `_run_paired_arm_async`, `run_openai_compatible`, `_extract_patch_async`. Keep the `TokenCount` dataclass (still used by telemetry) in a new file `bench/token_count.py`.

---

## Task 1: Verify SWE-agent install + basic sample task

**Files:**
- Modify: `bench/pyproject.toml` (add `sweagent`, `mcp`)
- Create: `bench/scripts/verify_sweagent.sh`

- [ ] **Step 1: Add dependencies to pyproject.toml**

Open `bench/pyproject.toml` and add to `[project].dependencies`:

```toml
"sweagent>=1.0",
"mcp>=1.0",
```

- [ ] **Step 2: Sync**

Run:
```bash
cd /home/adam/Documents/blastguard/bench && uv sync
```
Expected: both packages resolve. If `sweagent` package name on PyPI differs (e.g., `swe-agent` with hyphen), adjust the dependency spec and retry.

- [ ] **Step 3: Write a verification script**

Create `bench/scripts/verify_sweagent.sh`:

```bash
#!/usr/bin/env bash
# bench/scripts/verify_sweagent.sh
# Confirms SWE-agent is importable and its CLI responds.
set -euo pipefail

cd "$(dirname "$0")/.."
bench/.venv/bin/python -c "import sweagent; print('sweagent version:', sweagent.__version__)"
bench/.venv/bin/sweagent --help | head -5
echo "sweagent verification OK"
```

- [ ] **Step 4: Run it**

```bash
chmod +x /home/adam/Documents/blastguard/bench/scripts/verify_sweagent.sh
bash /home/adam/Documents/blastguard/bench/scripts/verify_sweagent.sh
```

Expected: prints a version number and the first 5 lines of `sweagent --help`. If the CLI entry point is different (`swe-agent`, `sweagent-run`, etc.), update the script to match — consult the SWE-agent repo README.

- [ ] **Step 5: Commit**

```bash
git add bench/pyproject.toml bench/scripts/verify_sweagent.sh bench/uv.lock
git commit -m "bench: add sweagent + mcp deps and verification script"
```

---

## Task 2: BlastGuard MCP bridge (Python client → Rust stdio server)

**Files:**
- Create: `bench/bundles/blastguard/bridge.py`
- Create: `bench/tests/test_bridge.py`

The bridge is a one-shot CLI: takes a tool name and JSON args on stdin/argv, spawns the `blastguard` binary (pointed at the workspace set by `$BLASTGUARD_PROJECT_ROOT`), sends one MCP `tools/call` request, prints the response, exits. No daemon.

- [x] **Step 1: Write failing test**

Create `bench/tests/test_bridge.py`:

```python
"""Bridge unit tests. Mocks the blastguard binary with a stub that emits
canned MCP responses over stdio.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest


def _fake_blastguard_script(tmp_path: Path) -> Path:
    """Write a Python script that mimics a minimal blastguard MCP server."""
    script = tmp_path / "fake_blastguard.py"
    script.write_text(
        'import json, sys\n'
        'for line in sys.stdin:\n'
        '    req = json.loads(line)\n'
        '    method = req.get("method")\n'
        '    if method == "initialize":\n'
        '        resp = {"jsonrpc":"2.0","id":req["id"],"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fake","version":"0"}}}\n'
        '    elif method == "tools/call":\n'
        '        name = req["params"]["name"]\n'
        '        resp = {"jsonrpc":"2.0","id":req["id"],"result":{"content":[{"type":"text","text":f"fake {name}"}]}}\n'
        '    else:\n'
        '        resp = {"jsonrpc":"2.0","id":req.get("id"),"result":{}}\n'
        '    sys.stdout.write(json.dumps(resp) + "\\n")\n'
        '    sys.stdout.flush()\n'
    )
    return script


def test_bridge_forwards_call_and_prints_text(tmp_path, monkeypatch):
    fake = _fake_blastguard_script(tmp_path)
    monkeypatch.setenv("BLASTGUARD_BINARY", f"{sys.executable} {fake}")
    monkeypatch.setenv("BLASTGUARD_PROJECT_ROOT", str(tmp_path))
    bridge = Path(__file__).parent.parent / "bundles" / "blastguard" / "bridge.py"
    args_json = json.dumps({"query": "callers of foo"})
    proc = subprocess.run(
        [sys.executable, str(bridge), "search", args_json],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert proc.returncode == 0, proc.stderr
    assert "fake search" in proc.stdout


def test_bridge_rejects_unknown_tool(tmp_path, monkeypatch):
    fake = _fake_blastguard_script(tmp_path)
    monkeypatch.setenv("BLASTGUARD_BINARY", f"{sys.executable} {fake}")
    monkeypatch.setenv("BLASTGUARD_PROJECT_ROOT", str(tmp_path))
    bridge = Path(__file__).parent.parent / "bundles" / "blastguard" / "bridge.py"
    proc = subprocess.run(
        [sys.executable, str(bridge), "nonsense_tool", "{}"],
        capture_output=True,
        text=True,
        timeout=10,
    )
    assert proc.returncode != 0
    assert "unknown tool" in proc.stderr.lower()
```

- [x] **Step 2: Confirm failure**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_bridge.py -v
```
Expected: FileNotFoundError or AttributeError — bridge.py doesn't exist yet.

- [x] **Step 3: Implement the bridge**

Create `bench/bundles/blastguard/bridge.py`:

```python
"""One-shot BlastGuard MCP bridge for SWE-agent bash wrappers.

Usage:
  python bridge.py <tool_name> <json_args>

Reads `$BLASTGUARD_BINARY` (space-separated; supports "python fake.py" in
tests) and `$BLASTGUARD_PROJECT_ROOT` (the workspace SWE-agent mounted).
Spawns the binary, sends initialize + tools/call, prints the tool's text
content to stdout, exits 0. On tool error, prints stderr and exits 2.
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys

ALLOWED_TOOLS = {"search", "apply_change", "run_tests"}


def _send(proc: subprocess.Popen, payload: dict) -> dict:
    assert proc.stdin is not None and proc.stdout is not None
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError("blastguard closed stdout unexpectedly")
    return json.loads(line)


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print("usage: bridge.py <tool_name> <json_args>", file=sys.stderr)
        return 2
    tool_name = argv[1]
    if tool_name not in ALLOWED_TOOLS:
        print(f"unknown tool: {tool_name!r}", file=sys.stderr)
        return 2
    try:
        args = json.loads(argv[2])
    except json.JSONDecodeError as e:
        print(f"invalid json args: {e}", file=sys.stderr)
        return 2

    binary_env = os.environ.get("BLASTGUARD_BINARY", "blastguard")
    project_root = os.environ.get("BLASTGUARD_PROJECT_ROOT")
    if not project_root:
        print("BLASTGUARD_PROJECT_ROOT env var is required", file=sys.stderr)
        return 2

    cmd = shlex.split(binary_env) + [project_root]
    proc = subprocess.Popen(
        cmd,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )

    try:
        _send(proc, {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "blastguard-bridge", "version": "0"},
            },
        })
        resp = _send(proc, {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": args},
        })
    finally:
        try:
            proc.stdin.close()  # type: ignore[union-attr]
        except Exception:  # noqa: BLE001
            pass
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()

    if "error" in resp:
        print(resp["error"], file=sys.stderr)
        return 2

    content = resp.get("result", {}).get("content", [])
    for block in content:
        if block.get("type") == "text":
            sys.stdout.write(block.get("text", ""))
    sys.stdout.write("\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
```

- [x] **Step 4: Confirm tests pass**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_bridge.py -v
```
Expected: 2 PASS.

- [x] **Step 5: Commit**

```bash
git add bench/bundles/blastguard/bridge.py bench/tests/test_bridge.py
git commit -m "bench: BlastGuard MCP bridge for SWE-agent bash wrappers"
```

---

## Task 3: Bundle shell wrappers + YAML config

**Files:**
- Create: `bench/bundles/blastguard/bg_search.sh`
- Create: `bench/bundles/blastguard/bg_apply_change.sh`
- Create: `bench/bundles/blastguard/bg_run_tests.sh`
- Create: `bench/bundles/blastguard/config.yaml`

- [x] **Step 1: Write the three shell wrappers**

Each is a thin exec of bridge.py with the tool name baked in. Arguments are JSON-quoted so SWE-agent can pass arbitrary payloads.

`bench/bundles/blastguard/bg_search.sh`:

```bash
#!/usr/bin/env bash
# Usage: bg_search.sh '<json args>'
# Expects $BLASTGUARD_BINARY and $BLASTGUARD_PROJECT_ROOT set by SWE-agent env.
set -euo pipefail
exec python "$(dirname "$0")/bridge.py" search "${1:-{\}}"
```

`bench/bundles/blastguard/bg_apply_change.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
exec python "$(dirname "$0")/bridge.py" apply_change "${1:-{\}}"
```

`bench/bundles/blastguard/bg_run_tests.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
exec python "$(dirname "$0")/bridge.py" run_tests "${1:-{\}}"
```

Make them executable:

```bash
chmod +x /home/adam/Documents/blastguard/bench/bundles/blastguard/bg_*.sh
```

- [x] **Step 1a: Directory layout per SWE-agent convention**

SWE-agent bundles live in a directory with:

```
bench/bundles/blastguard/
├── bin/
│   ├── blastguard_search
│   ├── blastguard_apply_change
│   └── blastguard_run_tests
├── config.yaml
└── bridge.py              # already created in Task 2
```

Executables MUST be under `bin/` with no extension (SWE-agent scans that subdir). Move `bg_search.sh` → `bin/blastguard_search` etc. Update the shell wrappers' `dirname` logic to account for the one-level-deeper path.

- [x] **Step 1b: Rewrite the three executables into `bin/`**

`bench/bundles/blastguard/bin/blastguard_search`:

```bash
#!/usr/bin/env bash
# Usage: blastguard_search '<json args>'
# $BLASTGUARD_BINARY + $BLASTGUARD_PROJECT_ROOT are set by the caller.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
exec python "$HERE/../bridge.py" search "${1:-{\}}"
```

`bench/bundles/blastguard/bin/blastguard_apply_change`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
exec python "$HERE/../bridge.py" apply_change "${1:-{\}}"
```

`bench/bundles/blastguard/bin/blastguard_run_tests`:

```bash
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
exec python "$HERE/../bridge.py" run_tests "${1:-{\}}"
```

Make them executable:

```bash
chmod +x /home/adam/Documents/blastguard/bench/bundles/blastguard/bin/*
```

- [x] **Step 2: Write the bundle config (real SWE-agent v1 schema)**

`bench/bundles/blastguard/config.yaml`:

```yaml
# SWE-agent v1 bundle declaring the three BlastGuard MCP tools.
# Referenced from a run config's `agent.tools.bundles` list (path-based).
# Shape matches tools/edit_anthropic/config.yaml in the SWE-agent repo.

tools:
  blastguard_search:
    signature: |
      blastguard_search <json_query>
    docstring: >
      Query BlastGuard's AST code graph. json_query is a single-quoted JSON
      string like '{"query": "callers of FOO"}' or '{"query": "tests for FILE"}'.
      Returns structured graph results in 50-300 tokens vs. 10k+ from grep.
      Strongly prefer this over bash grep when searching for symbol
      relationships (callers, callees, imports, tests, outline).
    arguments:
      - name: json_query
        type: string
        description: "JSON payload. Keys: 'query' (the natural-language query)."
        required: true

  blastguard_apply_change:
    signature: |
      blastguard_apply_change <json_changes>
    docstring: >
      Apply edits to a file with cascade-warning analysis. json_changes is a
      JSON string like '{"file": "path", "changes": [{"old_text": "...",
      "new_text": "..."}]}'. Returns SIGNATURE / ASYNC_CHANGE / ORPHAN /
      INTERFACE_BREAK warnings plus callers + tests context. Writes
      immediately. Prefer this over str_replace_editor for source-code edits.
    arguments:
      - name: json_changes
        type: string
        description: "JSON payload. Keys: 'file' (path), 'changes' (list of {old_text, new_text})."
        required: true

  blastguard_run_tests:
    signature: |
      blastguard_run_tests <json_opts>
    docstring: >
      Run the project's test suite (auto-detects pytest/jest/cargo).
      json_opts is a JSON string like '{"path": "optional/subpath"}'.
      Failures carry "YOU MODIFIED X (N edits ago)" annotations — use this to
      attribute a regression to your own recent edit.
    arguments:
      - name: json_opts
        type: string
        description: "JSON payload. Keys: 'path' (optional subpath to scope tests to)."
        required: false
```

- [x] **Step 3: Smoke-test the wrappers manually**

Build the release binary first (if not already built):

```bash
cd /home/adam/Documents/blastguard && cargo build --release
```

Then:

```bash
cd /home/adam/Documents/blastguard
export BLASTGUARD_BINARY="$(pwd)/target/release/blastguard"
export BLASTGUARD_PROJECT_ROOT="$(pwd)"
bench/.venv/bin/python bench/bundles/blastguard/bridge.py search '{"query":"outline of src/main.rs"}'
```

Expected: a short outline of `src/main.rs` prints to stdout, exit 0. If the binary isn't built or the path is wrong, the error will explain.

- [x] **Step 4: Commit**

```bash
git add bench/bundles/blastguard/
git commit -m "bench: BlastGuard bundle (config.yaml + 3 shell wrappers)"
```

---

## Task 4: SWE-agent orchestrator (one arm per invocation)

**Files:**
- Create: `bench/sweagent_runner.py`
- Create: `bench/tests/test_sweagent_runner.py`

This module owns the subprocess boundary. It exposes `run_arm(arm, task, model, seed, workspace, budget, telemetry_path)` — which assembles the SWE-agent CLI invocation, sets the BG env vars on arm=blastguard, parses the trajectory JSON for token/turn counts, returns a patch string.

- [x] **Step 1: Write failing test**

Create `bench/tests/test_sweagent_runner.py`:

```python
"""sweagent_runner tests. Mocks sweagent as a fake binary that writes a
predictable trajectory JSON and prints a fake patch.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest


@pytest.fixture
def fake_sweagent(tmp_path: Path, monkeypatch):
    """Install a fake sweagent binary that echoes args + writes a trajectory."""
    fake = tmp_path / "fake_sweagent.py"
    fake.write_text(
        'import json, sys, os, pathlib\n'
        'out_dir = pathlib.Path(os.environ["FAKE_SWEAGENT_OUT"])\n'
        'out_dir.mkdir(parents=True, exist_ok=True)\n'
        '(out_dir / "trajectory.json").write_text(json.dumps({\n'
        '    "instance_id": os.environ.get("FAKE_TASK_ID", "t1"),\n'
        '    "model_stats": {"prompt_tokens": 12345, "completion_tokens": 678, "n_turns": 4},\n'
        '    "model_patch": "diff --git a/foo b/foo\\n+added\\n",\n'
        '    "args": sys.argv[1:],\n'
        '}))\n'
        'sys.exit(0)\n'
    )
    monkeypatch.setenv("SWEAGENT_BINARY", f"{sys.executable} {fake}")
    return fake


def test_run_arm_returns_patch_and_token_count(fake_sweagent, tmp_path, monkeypatch):
    from bench.sweagent_runner import run_arm
    from bench.tasks import Task

    out_dir = tmp_path / "out"
    monkeypatch.setenv("FAKE_SWEAGENT_OUT", str(out_dir))
    monkeypatch.setenv("FAKE_TASK_ID", "demo__1")

    task = Task(
        task_id="demo__1",
        repo="demo/demo",
        base_commit="abc123",
        problem_statement="fix x",
        fail_to_pass=[], pass_to_pass=[], language="python", dockerhub_tag="tag",
    )
    res = run_arm(
        arm="raw",
        task=task,
        model="minimax/minimax-m2.7",
        seed=42,
        workspace=tmp_path / "work",
        output_dir=out_dir,
    )
    assert res.patch.startswith("diff --git")
    assert res.tokens.input == 12345
    assert res.tokens.output == 678
    assert res.tokens.turns == 4


def test_run_arm_blastguard_sets_env_vars(fake_sweagent, tmp_path, monkeypatch):
    from bench.sweagent_runner import run_arm
    from bench.tasks import Task

    out_dir = tmp_path / "out-bg"
    monkeypatch.setenv("FAKE_SWEAGENT_OUT", str(out_dir))
    monkeypatch.setenv("FAKE_TASK_ID", "demo__2")
    monkeypatch.setenv("BLASTGUARD_BINARY", "/fake/bg")

    task = Task(
        task_id="demo__2", repo="demo/demo", base_commit="abc",
        problem_statement="x", fail_to_pass=[], pass_to_pass=[],
        language="python", dockerhub_tag="t",
    )
    res = run_arm(
        arm="blastguard", task=task, model="m", seed=1,
        workspace=tmp_path / "w", output_dir=out_dir,
    )
    # The fake sweagent echoes argv into trajectory.json — verify the
    # bundle path was appended when arm=blastguard.
    traj = json.loads((out_dir / "trajectory.json").read_text())
    joined = " ".join(traj["args"])
    assert "blastguard" in joined
```

- [x] **Step 2: Confirm failure**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_sweagent_runner.py -v
```
Expected: ModuleNotFoundError.

- [x] **Step 3: Implement `bench/sweagent_runner.py`**

```python
"""Orchestrate SWE-agent subprocess invocations per task per arm.

SWE-agent writes a trajectory.json per run with `model_stats` (tokens,
turns) and `model_patch` (unified diff). We parse that file; we do not
scrape stdout.
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

from bench.tasks import Task
from bench.telemetry import TelemetryRecord
from bench.token_count import TokenCount

REPO_ROOT = Path(__file__).resolve().parent.parent
BUNDLE_PATH = REPO_ROOT / "bench" / "bundles" / "blastguard"


@dataclass(frozen=True, slots=True)
class ArmResult:
    patch: str
    tokens: TokenCount
    trajectory_path: Path


def _sweagent_cmd() -> list[str]:
    override = os.environ.get("SWEAGENT_BINARY")
    if override:
        return shlex.split(override)
    return ["sweagent", "run"]


def run_arm(
    *,
    arm: str,
    task: Task,
    model: str,
    seed: int,
    workspace: Path,
    output_dir: Path,
    timeout_seconds: int = 1800,
    blastguard_binary: Path | None = None,
) -> ArmResult:
    """Invoke SWE-agent once on `task`. Returns ArmResult."""
    if arm not in {"raw", "blastguard"}:
        raise ValueError(f"unknown arm: {arm!r}")

    output_dir.mkdir(parents=True, exist_ok=True)
    workspace.mkdir(parents=True, exist_ok=True)

    # Base CLI args. Mirrors SWE-agent's `run` subcommand; the exact flag
    # names may vary by SWE-agent version — adjust after Task 1's
    # verification shows the real CLI surface.
    args = [
        *_sweagent_cmd(),
        "--instance.repo", task.repo,
        "--instance.base_commit", task.base_commit,
        "--instance.problem_statement", task.problem_statement,
        "--instance.instance_id", task.task_id,
        "--agent.model.name", model,
        "--agent.model.temperature", "0",
        "--agent.model.per_instance_cost_limit", "5.00",
        "--output_dir", str(output_dir),
        "--workspace", str(workspace),
    ]

    env = os.environ.copy()
    # OpenRouter routing for OpenAI-compatible SWE-agent model calls.
    env.setdefault("OPENAI_API_BASE", "https://openrouter.ai/api/v1")
    if os.environ.get("OPENROUTER_API_KEY"):
        env["OPENAI_API_KEY"] = os.environ["OPENROUTER_API_KEY"]

    if arm == "blastguard":
        args += ["--tools.bundles", f"{BUNDLE_PATH}"]
        env["BLASTGUARD_PROJECT_ROOT"] = str(workspace)
        if blastguard_binary is not None:
            env["BLASTGUARD_BINARY"] = str(blastguard_binary)

    if seed:
        args += ["--agent.model.extra_args", json.dumps({"seed": seed})]

    proc = subprocess.run(
        args,
        env=env,
        timeout=timeout_seconds,
        capture_output=True,
        text=True,
        check=False,
    )
    traj_path = output_dir / "trajectory.json"
    if not traj_path.exists():
        raise RuntimeError(
            f"sweagent exited with code {proc.returncode} and wrote no "
            f"trajectory.json. stderr:\n{proc.stderr[:1000]}"
        )

    traj = json.loads(traj_path.read_text())
    stats = traj.get("model_stats", {})
    patch = str(traj.get("model_patch", ""))
    tokens = TokenCount(
        input=int(stats.get("prompt_tokens", 0)),
        cached_input=int(stats.get("cached_tokens", 0)),
        output=int(stats.get("completion_tokens", 0)),
        turns=int(stats.get("n_turns", 0)),
    )
    return ArmResult(patch=patch, tokens=tokens, trajectory_path=traj_path)
```

- [x] **Step 3a: Extract TokenCount into its own module**

Create `bench/token_count.py` (this exists today inside `bench/agent_loop.py`; we're moving it):

```python
"""Shared per-rollout token count dataclass."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class TokenCount:
    """Aggregated token usage for a single task rollout."""

    input: int
    cached_input: int
    output: int
    turns: int
```

- [x] **Step 4: Confirm tests pass**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_sweagent_runner.py -v
```
Expected: 2 PASS.

- [x] **Step 5: Commit**

```bash
git add bench/sweagent_runner.py bench/token_count.py bench/tests/test_sweagent_runner.py
git commit -m "bench: SWE-agent per-arm orchestrator + shared TokenCount"
```

---

## Task 5: Rewrite bench/runner.py around sweagent_runner

**Files:**
- Modify: `bench/runner.py`

- [x] **Step 1: Replace runner.py**

Overwrite `bench/runner.py` with:

```python
"""Run one arm against the SWE-bench Pro Python subset via SWE-agent.

Emits:
  results/<run_id>/patches.json            — evaluator input
  results/<run_id>/telemetry.jsonl         — per-task telemetry
  results/<run_id>/trajectories/<tid>/     — SWE-agent per-task output dirs
  results/<run_id>/config.json             — arm, seed, model, budget
"""

from __future__ import annotations

import argparse
import json
import random
import time
from pathlib import Path

from bench.budget import Budget, BudgetExceeded
from bench.evaluator import write_patches_json
from bench.sweagent_runner import run_arm
from bench.tasks import load_tasks
from bench.telemetry import TelemetryRecord, append_jsonl


def _results_dir(run_id: str) -> Path:
    d = Path(__file__).parent / "results" / run_id
    d.mkdir(parents=True, exist_ok=True)
    return d


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--arm", choices=("raw", "blastguard"), required=True)
    p.add_argument("--model", default="minimax/minimax-m2.7")
    p.add_argument("--limit", type=int, default=None)
    p.add_argument("--seed", type=int, default=42)
    p.add_argument("--budget-usd", type=float, required=True)
    p.add_argument("--run-id", required=True)
    p.add_argument("--in-price", type=float, default=0.30)
    p.add_argument("--out-price", type=float, default=1.20)
    p.add_argument("--cache-price", type=float, default=0.075)
    p.add_argument("--blastguard-binary", type=Path, default=None)
    p.add_argument(
        "--per-task-timeout",
        type=int,
        default=1800,
        help="Seconds before SWE-agent is killed per task",
    )
    args = p.parse_args()

    random.seed(args.seed)

    run_dir = _results_dir(args.run_id)
    (run_dir / "config.json").write_text(
        json.dumps(
            {
                "arm": args.arm,
                "model": args.model,
                "seed": args.seed,
                "budget_usd": args.budget_usd,
                "limit": args.limit,
                "per_task_timeout": args.per_task_timeout,
            },
            indent=2,
        )
    )

    budget = Budget(cap_usd=args.budget_usd)
    tasks = load_tasks(limit=args.limit, python_only=True)
    tasks.sort(key=lambda t: t.task_id)  # identical order both arms

    telemetry_path = run_dir / "telemetry.jsonl"
    predictions: list[tuple[str, str]] = []
    trajectories_dir = run_dir / "trajectories"
    workspaces_dir = run_dir / "workspaces"

    for task in tasks:
        t0 = time.time()
        task_traj = trajectories_dir / task.task_id
        task_ws = workspaces_dir / task.task_id
        patch = ""
        tokens_input = tokens_cached = tokens_output = tokens_turns = 0
        cost = 0.0
        error: str | None = None
        try:
            res = run_arm(
                arm=args.arm,
                task=task,
                model=args.model,
                seed=args.seed,
                workspace=task_ws,
                output_dir=task_traj,
                timeout_seconds=args.per_task_timeout,
                blastguard_binary=args.blastguard_binary,
            )
            patch = res.patch
            tokens_input = res.tokens.input
            tokens_cached = res.tokens.cached_input
            tokens_output = res.tokens.output
            tokens_turns = res.tokens.turns
            cost = budget.record(
                input_tokens=tokens_input,
                cached_input_tokens=tokens_cached,
                output_tokens=tokens_output,
                in_price_per_m=args.in_price,
                cache_read_per_m=args.cache_price,
                out_price_per_m=args.out_price,
            )
        except BudgetExceeded as e:
            print(f"[{task.task_id}] BUDGET STOP: {e}")
            break
        except Exception as e:  # noqa: BLE001
            error = f"{type(e).__name__}: {e}"
            print(f"[{task.task_id}] ERROR: {error}")

        predictions.append((task.task_id, patch))
        append_jsonl(
            TelemetryRecord(
                task_id=task.task_id,
                arm=args.arm,
                input_tokens=tokens_input,
                cached_input_tokens=tokens_cached,
                output_tokens=tokens_output,
                turns=tokens_turns,
                wall_seconds=time.time() - t0,
                cost_usd=cost,
                patch_bytes=len(patch.encode("utf-8")),
                error=error,
            ),
            telemetry_path,
        )
        print(
            f"[{task.task_id}] turns={tokens_turns} tokens={tokens_input+tokens_output} "
            f"cost=${cost:.4f} spent=${budget.spent_usd:.4f}"
        )

    write_patches_json(
        predictions,
        prefix=f"{args.arm}-{args.model.replace('/', '_')}",
        out_path=run_dir / "patches.json",
    )
    print(f"done: wrote {len(predictions)} predictions; spent ${budget.spent_usd:.4f}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

- [x] **Step 2: Dry-run smoke**

```bash
cd /home/adam/Documents/blastguard
bench/.venv/bin/python -m bench.runner \
  --arm raw --limit 0 --seed 42 \
  --budget-usd 0.10 --run-id dry --model minimax/minimax-m2.7
```

Expected: exit 0, `bench/results/dry/config.json` emitted, empty telemetry.jsonl and patches.json (array `[]`). No SWE-agent invoked because `--limit 0`.

- [x] **Step 3: Commit**

```bash
git add bench/runner.py
git commit -m "bench: wire runner.py to sweagent_runner per arm"
```

---

## Task 6: Retire agent_loop.py dead code

**Files:**
- Modify: `bench/agent_loop.py` (strip everything except imports if any callers remain)
- Delete (if no callers): `bench/agent_loop.py`

- [ ] **Step 1: Check for callers**

```bash
cd /home/adam/Documents/blastguard && grep -r --include='*.py' 'from bench.agent_loop\|import agent_loop' bench/ || echo 'no importers'
```

Expected: `no importers` OR only `bench/tests/` importing `TokenCount` — which we've moved. Update any test imports to `from bench.token_count import TokenCount`.

- [ ] **Step 2: Delete the file and its test (if present)**

```bash
cd /home/adam/Documents/blastguard && git rm bench/agent_loop.py bench/tests/test_agent_loop.py 2>/dev/null || true
```

(The `|| true` handles missing test file without blowing up.)

- [ ] **Step 3: Verify tests still pass**

```bash
cd /home/adam/Documents/blastguard/bench && HF_HOME=/tmp/hf uv run pytest -v
```

Expected: all prior tests still pass, plus the new tests from Tasks 2 and 4. Fix any import errors by retargeting `TokenCount` to its new home.

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "bench: retire custom agent loop; SWE-agent is now the scaffold"
```

---

## Task 7: Update compare.py to treat SWE-agent timeouts as infra_failure

**Files:**
- Modify: `bench/evaluator.py` (extend `parse_evaluator_output` to treat `timeout_during_generation: true` records as infra_failure)
- Modify: `bench/tests/test_evaluator.py` (add a test)

SWE-agent can emit `trajectory.json` with `exit_status: "exit_timeout"` when a task runs out of time. We should NOT count those as real task failures in the paired A/B; they belong in the infra_failure bucket (same as rate limits and evaluator crashes).

This only affects our pre-evaluator patch generation. The `SWE-bench_Pro-os` evaluator already classifies patch-less tasks as unresolved; we need to flag them earlier so `pair_results()` drops them symmetrically.

- [ ] **Step 1: Add the test**

Append to `bench/tests/test_evaluator.py`:

```python
def test_parse_evaluator_output_flags_empty_patch(tmp_path: Path):
    """An evaluator entry with empty model_patch (upstream timeout) is infra_failure."""
    out_dir = tmp_path / "out"
    out_dir.mkdir()
    (out_dir / "django__999.json").write_text(
        json.dumps({
            "instance_id": "django__999",
            "resolved": False,
            "model_patch": "",
        })
    )
    results = parse_evaluator_output(out_dir)
    r = results[0]
    assert r.resolved is False
    assert r.infra_failure is True
    assert "empty_patch" in (r.raw.get("error") or "")
```

- [ ] **Step 2: Confirm failure**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_evaluator.py -v
```
Expected: the new test FAILS — current `parse_evaluator_output` doesn't check for empty patches.

- [ ] **Step 3: Extend `parse_evaluator_output`**

Open `bench/evaluator.py`. In the `for path in sorted(...)` loop, inside the `else` branch that builds the `EvaluatorResult`, change the logic:

```python
task_id = str(payload.get("instance_id", path.stem))
has_error = bool(payload.get("error")) or "resolved" not in payload
empty_patch = not payload.get("model_patch", "").strip()
infra_failure = has_error or empty_patch
resolved = bool(payload.get("resolved", False)) if not infra_failure else False
if empty_patch and not has_error:
    payload = {**payload, "error": "empty_patch (upstream generation failure)"}

results.append(
    EvaluatorResult(
        task_id=task_id,
        resolved=resolved,
        infra_failure=infra_failure,
        raw=payload,
    )
)
```

- [ ] **Step 4: Confirm all tests pass**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_evaluator.py -v
```
Expected: 4 PASS (3 existing + 1 new).

- [ ] **Step 5: Commit**

```bash
git add bench/evaluator.py bench/tests/test_evaluator.py
git commit -m "bench: treat empty-patch outputs as infra_failure in paired analysis"
```

---

## Task 8: Rate-limit + retry guard for SWE-agent runs

**Files:**
- Modify: `bench/sweagent_runner.py`
- Modify: `bench/tests/test_sweagent_runner.py`

When OpenRouter returns 429, SWE-agent's underlying LiteLLM call surfaces it as a non-zero exit. We want to retry the task once after a 60-second cooldown rather than burning it. One retry is the cap — two is a loop trap.

- [ ] **Step 1: Add the test**

Append to `bench/tests/test_sweagent_runner.py`:

```python
def test_run_arm_retries_once_on_rate_limit(fake_sweagent, tmp_path, monkeypatch):
    """First invocation exits with rate-limit marker; second succeeds."""
    import sys
    from bench.sweagent_runner import run_arm
    from bench.tasks import Task

    out_dir = tmp_path / "out-rl"
    monkeypatch.setenv("FAKE_SWEAGENT_OUT", str(out_dir))
    monkeypatch.setenv("FAKE_TASK_ID", "demo__rl")

    # Install a fake that fails on first call, succeeds on second.
    counter_file = tmp_path / "count.txt"
    counter_file.write_text("0")
    fake = tmp_path / "fake_sweagent_rl.py"
    fake.write_text(
        'import json, sys, os, pathlib\n'
        'cf = pathlib.Path(os.environ["COUNTER_FILE"])\n'
        'n = int(cf.read_text())\n'
        'cf.write_text(str(n + 1))\n'
        'if n == 0:\n'
        '    sys.stderr.write("RateLimitError: 429")\n'
        '    sys.exit(1)\n'
        'out = pathlib.Path(os.environ["FAKE_SWEAGENT_OUT"])\n'
        'out.mkdir(parents=True, exist_ok=True)\n'
        '(out / "trajectory.json").write_text(json.dumps({\n'
        '    "instance_id": "demo__rl",\n'
        '    "model_stats": {"prompt_tokens": 1, "completion_tokens": 1, "n_turns": 1},\n'
        '    "model_patch": "diff\\n",\n'
        '    "args": [],\n'
        '}))\n'
        'sys.exit(0)\n'
    )
    monkeypatch.setenv("SWEAGENT_BINARY", f"{sys.executable} {fake}")
    monkeypatch.setenv("COUNTER_FILE", str(counter_file))
    monkeypatch.setenv("BENCH_RATE_LIMIT_SLEEP", "0")  # skip real sleep in tests

    task = Task(
        task_id="demo__rl", repo="r", base_commit="c",
        problem_statement="p", fail_to_pass=[], pass_to_pass=[],
        language="python", dockerhub_tag="t",
    )
    res = run_arm(
        arm="raw", task=task, model="m", seed=1,
        workspace=tmp_path / "w", output_dir=out_dir,
    )
    assert res.patch.startswith("diff")
    assert int(counter_file.read_text()) == 2  # called exactly twice
```

- [ ] **Step 2: Confirm failure**

Expected: FAIL — current `run_arm` only invokes once.

- [ ] **Step 3: Wrap the subprocess call with retry logic**

In `bench/sweagent_runner.py`, replace the `proc = subprocess.run(args, ...)` block with:

```python
    import time  # noqa: PLC0415

    rate_limit_sleep = int(os.environ.get("BENCH_RATE_LIMIT_SLEEP", "60"))
    last_stderr = ""
    for attempt in (1, 2):
        proc = subprocess.run(
            args,
            env=env,
            timeout=timeout_seconds,
            capture_output=True,
            text=True,
            check=False,
        )
        traj_path = output_dir / "trajectory.json"
        if traj_path.exists():
            break
        last_stderr = proc.stderr
        if attempt == 1 and "rate" in proc.stderr.lower():
            time.sleep(rate_limit_sleep)
            continue
        break
    if not traj_path.exists():
        raise RuntimeError(
            f"sweagent exited with code {proc.returncode} and wrote no "
            f"trajectory.json after {attempt} attempt(s). stderr:\n"
            f"{last_stderr[:1000]}"
        )
```

- [ ] **Step 4: Confirm all tests pass**

```bash
cd /home/adam/Documents/blastguard/bench && uv run pytest tests/test_sweagent_runner.py -v
```
Expected: 3 PASS.

- [ ] **Step 5: Commit**

```bash
git add bench/sweagent_runner.py bench/tests/test_sweagent_runner.py
git commit -m "bench: one-shot retry on rate-limit exits from sweagent"
```

---

## Task 9: End-to-end paired smoke (3 tasks, both arms) — LOCAL DRY-RUN ONLY

**Files:**
- No code changes.

This is a harness-wiring smoke, not a paid run. We use `--dry-run` by mocking `SWEAGENT_BINARY` to a script that emits canned trajectories. No API keys, no Docker pulls, no spend.

- [ ] **Step 1: Write a local-mock SWE-agent script**

```bash
cat > /tmp/mock_sweagent.py << 'PYEOF'
import json, os, sys, pathlib
out = pathlib.Path([a for a in sys.argv if a.startswith('--output_dir=') or False] or ['--output_dir='])[-1]
# Simpler: find --output_dir N argv pair
args = sys.argv[1:]
out_dir = None
for i, a in enumerate(args):
    if a == "--output_dir" and i + 1 < len(args):
        out_dir = pathlib.Path(args[i + 1])
if out_dir is None:
    sys.stderr.write("no --output_dir in argv\n"); sys.exit(2)
out_dir.mkdir(parents=True, exist_ok=True)
iid = "unknown"
for i, a in enumerate(args):
    if a == "--instance.instance_id" and i + 1 < len(args):
        iid = args[i + 1]
(out_dir / "trajectory.json").write_text(json.dumps({
    "instance_id": iid,
    "model_stats": {"prompt_tokens": 50000, "completion_tokens": 5000, "n_turns": 12},
    "model_patch": f"diff --git a/readme b/readme\n+ {iid}\n",
}))
sys.exit(0)
PYEOF
```

- [ ] **Step 2: Run raw arm against 3 tasks via mock**

```bash
cd /home/adam/Documents/blastguard && \
  export SWEAGENT_BINARY="$(bench/.venv/bin/python -c 'import sys;print(sys.executable)') /tmp/mock_sweagent.py" && \
  HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
    --arm raw --limit 3 --seed 42 \
    --budget-usd 1.00 --run-id mock-raw \
    --model minimax/minimax-m2.7
```

Expected: completes instantly, `bench/results/mock-raw/patches.json` has 3 entries, telemetry shows 50k/5k tokens per task, spend ~$0.06.

- [ ] **Step 3: Run blastguard arm against the same 3 tasks**

```bash
cd /home/adam/Documents/blastguard && \
  export SWEAGENT_BINARY="$(bench/.venv/bin/python -c 'import sys;print(sys.executable)') /tmp/mock_sweagent.py" && \
  HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
    --arm blastguard --limit 3 --seed 42 \
    --budget-usd 1.00 --run-id mock-bg \
    --model minimax/minimax-m2.7
```

Expected: same shape, 3 entries.

- [ ] **Step 4: Exercise compare.py on the mock outputs**

The mock emits trajectories, not evaluator output JSON. Skip the evaluator for this smoke — we'll test `compare.py` separately with synthesized evaluator JSON in the existing tests. For this task, just verify both `patches.json` files are well-formed:

```bash
cd /home/adam/Documents/blastguard && \
  bench/.venv/bin/python -c "
import json, pathlib
for run in ('mock-raw', 'mock-bg'):
    p = json.loads((pathlib.Path('bench/results') / run / 'patches.json').read_text())
    print(run, 'tasks:', [e['instance_id'] for e in p])
    assert len(p) == 3, f'{run} has {len(p)} entries, expected 3'
"
```

Expected: prints both runs with 3 task IDs each. No assertion errors.

- [ ] **Step 5: Decision gate**

Before any paid invocation:
- [ ] Both arms produced 3 patches via the mock
- [ ] Telemetry JSONL is well-formed (every row has `task_id`, `arm`, token counts, `cost_usd`)
- [ ] `bench/results/mock-*/trajectories/<tid>/trajectory.json` exists per task
- [ ] No unexpected files under `bench/results/` (check gitignore covers this)

If any fails, STOP and fix before proceeding to real runs.

- [ ] **Step 6: Clean up mock outputs**

```bash
rm -rf /home/adam/Documents/blastguard/bench/results/mock-raw /home/adam/Documents/blastguard/bench/results/mock-bg /tmp/mock_sweagent.py
```

---

## Task 10: Update bench/README.md with the new workflow

**Files:**
- Modify: `bench/README.md`

- [ ] **Step 1: Replace workflow section**

Read the existing `bench/README.md`. Find the `## Workflow (Plan 8)` section and rewrite it as `## Workflow (Plan 9 — SWE-agent scaffold)` containing:

```markdown
## Workflow (Plan 9 — SWE-agent scaffold)

BlastGuard runs as a SWE-agent bundle. SWE-agent handles workspace
cloning, tool dispatch, patch extraction, and timeouts. We orchestrate
per-arm invocations and do paired McNemar's analysis on the outputs.

### Prerequisites

- Docker daemon running (for SWE-agent's Docker mode + the SWE-bench_Pro-os evaluator)
- `.env` with `OPENROUTER_API_KEY=sk-or-v1-...`
- `bench/.evaluator/` cloned via `bash bench/scripts/clone_evaluator.sh`
- `target/release/blastguard` built (`cargo build --release`)
- SWE-agent installed (`uv sync` inside `bench/`)

### 1. Harness mock-smoke (no spend, local only)

Run Task 9 from Plan 9 — proves the orchestrator wiring works without
touching OpenRouter credits. Expected: ~$0 spend, both arms produce
3 mock predictions each.

### 2. Paired 10-task smoke (~$3-4)

    cd /home/adam/Documents/blastguard
    export BLASTGUARD_BIN="$(pwd)/target/release/blastguard"

    # raw arm
    HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
      --arm raw --limit 10 --seed 42 \
      --budget-usd 5.00 --run-id smoke-raw \
      --model minimax/minimax-m2.7

    # blastguard arm (same seed, same tasks)
    HF_HOME=/tmp/hf bench/.venv/bin/python -m bench.runner \
      --arm blastguard --limit 10 --seed 42 \
      --budget-usd 5.00 --run-id smoke-bg \
      --model minimax/minimax-m2.7 \
      --blastguard-binary "$BLASTGUARD_BIN"

### 3. Grade both arms

    bench/.venv/bin/python -c "
    from bench.evaluator import run_evaluator
    from pathlib import Path
    for rid in ('smoke-raw', 'smoke-bg'):
        run_evaluator(
            evaluator_dir=Path('bench/.evaluator'),
            raw_sample_csv=Path('bench/.evaluator/swe_bench_pro_full.csv'),
            patches_json=Path(f'bench/results/{rid}/patches.json'),
            output_dir=Path(f'bench/results/{rid}/eval'),
            num_workers=2,
            timeout_seconds=3600,
        )
    "

### 4. Paired comparison

    bench/.venv/bin/python -m bench.compare \
      --raw-output-dir bench/results/smoke-raw/eval \
      --blastguard-output-dir bench/results/smoke-bg/eval

### 5. Pilot → full run gating

- 100-task pilot: ~$38 (raw ~$23, BG ~$14). Proceed to full run only if delta ≥ +1pp trending positive.
- 731-task full: ~$275 (raw ~$170, BG ~$105).

Budget per `--budget-usd` is a hard ceiling — the runner aborts mid-run
if the next call would exceed it.
```

- [ ] **Step 2: Commit**

```bash
git add bench/README.md
git commit -m "bench: document Plan 9 SWE-agent workflow"
```

---

## Self-Review

**Spec coverage:**
- Goal: adopt SWE-agent + integrate BlastGuard as a bundle → Tasks 1-4 own this.
- Replace the broken custom loop → Tasks 5, 6.
- Preserve paired-analysis rigor → Task 7 extends existing evaluator logic; compare.py unchanged (it's scaffold-agnostic).
- Handle rate limits safely → Task 8.
- Mock-smoke gate before spend → Task 9.
- Documentation → Task 10.

**Placeholder scan:**
- Every step has concrete code or an explicit "no-op" rationale (e.g., Task 9 Step 4 explains why we skip the evaluator).
- `TokenCount` is moved in Task 4 Step 3a; test imports in Task 6 Step 1 are routed to the new location.
- No "TBD" / "implement later" items.

**Type consistency:**
- `TokenCount(input, cached_input, output, turns)` matches between Task 4, Task 5, Task 8, and Plan 8's `TelemetryRecord`.
- `ArmResult(patch, tokens, trajectory_path)` introduced in Task 4, consumed in Task 5 — matches.
- `EvaluatorResult(task_id, resolved, infra_failure, raw)` extended in Task 7 — field shape preserved.

**Known risk the plan accepts on purpose:**
- SWE-agent's CLI flag names (`--instance.repo`, `--agent.model.name`, `--tools.bundles`) are inferred from the researcher's summary. Task 1 includes a step to verify and adjust before Task 4 depends on them. Plan assumes `sweagent` is installable via `uv add sweagent`; if the PyPI package is named differently, Task 1 Step 2 will surface it.

---

## Execution

Per user preference ("Subagent-Driven → always"): dispatch fresh subagents per task via `superpowers:subagent-driven-development`, no mode prompt. Task 1 runs first; Tasks 2-3 independent of 4+; Tasks 4-8 must run in order (each depends on the prior).
