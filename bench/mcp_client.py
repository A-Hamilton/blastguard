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

_REPO_ROOT = Path(__file__).resolve().parent.parent


class BlastGuardClient:
    """Synchronous lifecycle wrapper for the BlastGuard MCP server process.

    Usage::

        client = BlastGuardClient()
        client.start()
        ...
        client.stop()

    The underlying process is blastguard running in stdio MCP mode against
    the project root.  The paired-arm runner only needs the process alive for
    system-prompt biasing; actual MCP session management happens inside the
    async agent loop when needed.
    """

    def __init__(self, project_root: Path | None = None) -> None:
        self._root = project_root or _REPO_ROOT
        self._proc: object | None = None  # subprocess.Popen if running

    def start(self) -> None:
        """Spawn the BlastGuard binary.  No-op if already running."""
        if self._proc is not None:
            return
        import subprocess  # noqa: PLC0415

        try:
            binary = find_blastguard_binary(self._root)
        except FileNotFoundError:
            # Binary not built yet — continue without it; agent gets raw arm
            # behaviour even when BlastGuard arm is requested.
            return
        self._proc = subprocess.Popen(  # noqa: S603
            [str(binary), str(self._root)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def stop(self) -> None:
        """Terminate the BlastGuard process if running."""
        if self._proc is not None:
            import subprocess  # noqa: PLC0415

            proc = self._proc
            self._proc = None
            try:
                proc.terminate()  # type: ignore[union-attr]
                proc.wait(timeout=5)  # type: ignore[union-attr]
            except subprocess.TimeoutExpired:
                proc.kill()  # type: ignore[union-attr]
            except OSError:
                pass


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
