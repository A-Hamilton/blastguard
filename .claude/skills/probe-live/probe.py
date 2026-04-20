#!/usr/bin/env python3
"""Probe a named BlastGuard fixture with a single search query.

Usage:
    probe.py <fixture_name> "<search query>"

Exit code:
    0 on success (hits printed to stdout, possibly empty with a hint)
    1 on user error (bad fixture name, missing query, binary not found)
    2 on MCP protocol error

This script is intentionally dependency-free — stdlib only.
"""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# Fixture registry. Each entry is a dict of {relative_path: file_body}.
# Keep fixtures tiny — 2-5 files each.
# ---------------------------------------------------------------------------

FIXTURES: dict[str, dict[str, str]] = {
    "python-relative": {
        "src/pkg/__init__.py": "",
        "src/pkg/sub/__init__.py": "",
        "src/pkg/sub/leaf.py": "def leaf():\n    return 1\n",
        "src/pkg/mid.py": "from .sub.leaf import leaf\ndef mid(): return leaf()\n",
        "src/pkg/sub/deep.py": "from ..mid import mid\ndef deep(): return mid()\n",
    },
    "python-absolute": {
        "src/utils/auth.py": "def login(user):\n    return user\n",
        "src/handler.py": (
            "from utils.auth import login\n\n"
            "def handle(req):\n    return login(req)\n"
        ),
    },
    "tsx-arrow-consts": {
        "src/Arrow.tsx": (
            "const Arrow = () => {\n"
            "    return <span>arrow</span>;\n"
            "};\n"
            "export default Arrow;\n"
        ),
        "src/App.tsx": (
            "import Arrow from './Arrow';\n"
            "export function App() {\n"
            "    return <div><Arrow /></div>;\n"
            "}\n"
        ),
    },
    "tsconfig-alias": {
        "tsconfig.json": (
            '{"compilerOptions": {"baseUrl": ".",'
            ' "paths": {"@shared/*": ["src/shared/*"]}}}\n'
        ),
        "src/shared/greet.ts": (
            "export function greet(name: string): string {\n"
            "    return `hi ${name}`;\n"
            "}\n"
        ),
        "src/app.ts": (
            'import { greet } from "@shared/greet";\n'
            'export function run(): string { return greet("x"); }\n'
        ),
    },
    "rust-siblings": {
        "Cargo.toml": (
            '[package]\nname = "probe"\nversion = "0.0.0"\nedition = "2021"\n'
        ),
        "src/lib.rs": "pub mod graph;\npub use graph::impact::flag;\n",
        "src/graph/mod.rs": "pub mod impact;\npub use impact::flag;\n",
        "src/graph/impact.rs": "pub fn flag(x: u32) -> u32 { x + 1 }\n",
    },
    "ts-relative": {
        "src/utils/auth.ts": (
            "export function login(user: string): string {\n"
            "    return user;\n"
            "}\n"
        ),
        "src/handler.ts": (
            'import { login } from "./utils/auth";\n\n'
            "export function handle(req: string): string {\n"
            "    return login(req);\n"
            "}\n"
        ),
    },
}


# ---------------------------------------------------------------------------


def find_binary() -> str:
    """Locate the release BlastGuard binary relative to CWD.

    Users typically run this skill from the project root, so we look for
    `target/release/blastguard` there first. If missing, surface a clear
    error rather than erroring obscurely in the MCP frame.
    """
    candidates = [
        Path.cwd() / "target" / "release" / "blastguard",
        Path("/home/adam/Documents/blastguard/target/release/blastguard"),
    ]
    for p in candidates:
        if p.is_file() and os.access(p, os.X_OK):
            return str(p)
    raise FileNotFoundError(
        "BlastGuard release binary not found. Run `cargo build --release` first "
        f"(looked in {', '.join(str(p) for p in candidates)})."
    )


def seed_fixture(fixture: dict[str, str], dest: Path) -> None:
    for rel, body in fixture.items():
        target = dest / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(body)
    # BlastGuard's walker activates gitignore only inside a git repo.
    # A bare `git init` is enough; no commit needed.
    subprocess.run(
        ["git", "init", "-q"],
        cwd=dest,
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def run_probe(binary: str, project_root: Path, query: str) -> dict:
    """Spawn BlastGuard, send three frames, return the tools/call result.

    Frames:
      1. initialize
      2. notifications/initialized
      3. tools/call name=search arguments={"query": <query>}
    """
    frames = [
        {
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "probe-live", "version": "0.1"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}},
        {
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "search", "arguments": {"query": query}},
        },
    ]
    payload = "\n".join(json.dumps(f) for f in frames) + "\n"
    try:
        proc = subprocess.run(
            [binary, str(project_root)],
            input=payload,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except subprocess.TimeoutExpired:
        raise RuntimeError("BlastGuard probe timed out after 30s") from None

    # Scan stdout for the response with id=2.
    for line in proc.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("id") == 2:
            return msg
    raise RuntimeError(
        f"no tools/call response in stdout. stderr: {proc.stderr[:500]}"
    )


def main() -> int:
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <fixture_name> '<query>'", file=sys.stderr)
        print(f"fixtures: {', '.join(sorted(FIXTURES))}", file=sys.stderr)
        return 1

    fixture_name, query = sys.argv[1], sys.argv[2]
    fixture = FIXTURES.get(fixture_name)
    if fixture is None:
        print(f"unknown fixture: {fixture_name!r}", file=sys.stderr)
        print(f"available: {', '.join(sorted(FIXTURES))}", file=sys.stderr)
        return 1

    try:
        binary = find_binary()
    except FileNotFoundError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1

    tmp = Path(tempfile.mkdtemp(prefix=f"probe-{fixture_name}-"))
    try:
        seed_fixture(fixture, tmp)
        try:
            response = run_probe(binary, tmp, query)
        except (RuntimeError, FileNotFoundError) as e:
            print(f"probe error: {e}", file=sys.stderr)
            return 2

        if "error" in response:
            print(f"MCP error: {response['error']}", file=sys.stderr)
            return 2

        hits = (
            response.get("result", {})
            .get("structuredContent", {})
            .get("hits", [])
        )
        print(f">>> fixture={fixture_name} query={query!r} → {len(hits)} hits")
        for h in hits:
            print(f"    {h}")
        print(f"(tempdir kept at {tmp})")
    finally:
        # Keep the tempdir around only on error — on clean success we
        # clean up immediately. Flip this if you want to inspect the
        # fixture after a successful run.
        if os.environ.get("PROBE_LIVE_KEEP") != "1":
            shutil.rmtree(tmp, ignore_errors=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
