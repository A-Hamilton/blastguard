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
