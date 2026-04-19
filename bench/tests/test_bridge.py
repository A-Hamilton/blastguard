"""Bridge unit tests. Mocks the blastguard binary with a stub that emits
canned MCP responses over stdio.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def _fake_blastguard_script(tmp_path: Path) -> Path:
    """Write a Python script that mimics a minimal blastguard MCP server."""
    script = tmp_path / "fake_blastguard.py"
    init_result = (
        '{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},'
        '"serverInfo":{"name":"fake","version":"0"}}'
    )
    script.write_text(
        'import json, sys\n'
        'for line in sys.stdin:\n'
        '    req = json.loads(line)\n'
        '    method = req.get("method")\n'
        '    if method == "initialize":\n'
        f'        resp = {{"jsonrpc":"2.0","id":req["id"],"result":{init_result}}}\n'
        '    elif method == "tools/call":\n'
        '        name = req["params"]["name"]\n'
        '        resp = {"jsonrpc":"2.0","id":req["id"],'
        '"result":{"content":[{"type":"text","text":f"fake {name}"}]}}\n'
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
