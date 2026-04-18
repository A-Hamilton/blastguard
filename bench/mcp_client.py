"""Thin async wrapper around the `mcp` SDK to talk to BlastGuard.

Spawns `target/release/blastguard <project_root>` as a subprocess and
connects over stdio. Exposes `list_tools()` and `call_tool(name, args)`
so the agent loop can forward tool-use requests.
"""

from __future__ import annotations

import shutil
from collections.abc import AsyncIterator
from contextlib import asynccontextmanager
from pathlib import Path

try:
    from mcp import ClientSession
    from mcp.client.stdio import StdioServerParameters, stdio_client
except ImportError:  # pragma: no cover
    ClientSession = None  # type: ignore[assignment]
    StdioServerParameters = None  # type: ignore[assignment]
    stdio_client = None  # type: ignore[assignment]


BLASTGUARD_BINARY_REL = "target/release/blastguard"


def find_blastguard_binary(repo_root: Path) -> Path:
    """Locate the compiled BlastGuard binary. Raise if missing."""
    candidate = repo_root / BLASTGUARD_BINARY_REL
    if candidate.is_file():
        return candidate
    # Fallback: first `blastguard` on PATH.
    which = shutil.which("blastguard")
    if which:
        return Path(which)
    raise FileNotFoundError(
        f"blastguard binary not found at {candidate} or on PATH. "
        "Run `cargo build --release` at the repo root first."
    )


@asynccontextmanager
async def blastguard_session(
    project_root: Path,
    blastguard_binary: Path,
) -> AsyncIterator[ClientSession]:
    """Async context manager yielding an open MCP ClientSession."""
    if ClientSession is None:
        raise RuntimeError("mcp SDK not installed — run `uv sync` in bench/")
    params = StdioServerParameters(
        command=str(blastguard_binary),
        args=[str(project_root)],
        env=None,
    )
    async with stdio_client(params) as (read, write), ClientSession(read, write) as session:
        await session.initialize()
        yield session
