"""Micro-A/B benchmark: native tools vs. native tools + BlastGuard MCP.

Runs N representative tasks against a single local repo (default: this one)
with two tool configurations:

    Arm A (raw):       Read, Grep, Bash  (Python implementations)
    Arm B (blastguard): Read, Grep, Bash + blastguard_{search,apply_change,run_tests}

Same model, same prompts, same seed. Measures per-task input/output tokens,
turns, wall seconds, and whether BlastGuard tools were used. Writes a JSONL
log and prints a summary table.

Not statistically powered — this is a 2-5 task smoke to replace "projected"
with "measured on N real tasks" in the README.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Tasks — real exploration questions against this repo.
# ---------------------------------------------------------------------------

TASKS = [
    {
        "id": "explore-cold-index",
        "prompt": (
            "Using the tools available, explore the BlastGuard Rust codebase at "
            "{project_root} and explain what the `cold_index` function does and "
            "what calls it. Answer in 3-5 sentences. When done, write 'DONE' "
            "on its own line."
        ),
    },
    {
        "id": "callers-apply-edit",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, find every caller of "
            "the `apply_edit` function. For each caller, briefly describe what "
            "it is (function name + file) and what kind of value it passes for "
            "the `old_text` argument. Answer concisely. Write 'DONE' when finished."
        ),
    },
    {
        # Multi-hop cross-file navigation — harder, closer to BlastGuard's
        # design sweet spot (even though Phase 1 caller edges are intra-file,
        # chain queries span multiple symbols).
        "id": "chain-search-to-graph",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, find the call chain "
            "from the MCP `search` tool entry point down into the code-graph "
            "module. In other words: when the MCP search tool is invoked, which "
            "intermediate function(s) get called on the way to the graph "
            "operations? Name each function (file + function name) in order. "
            "Keep the answer under 10 lines. Write 'DONE' when finished."
        ),
    },
    {
        # Cascade/blast-radius question — exactly what apply_change is
        # designed to answer but we're asking it without telling the model
        # "use apply_change". Reveals whether the model reaches for it
        # unprompted.
        "id": "cascade-signature-change",
        "prompt": (
            "In the BlastGuard Rust repo at {project_root}, suppose we wanted "
            "to change the signature of `apply_edit` to take a single `Edit` "
            "struct instead of three separate `&Path`, `&str`, `&str` "
            "arguments. List every function that would need to be updated, "
            "and explain why. Keep the answer concise — just a bulleted list "
            "with the file:line of each caller and a one-line reason. "
            "Write 'DONE' when finished."
        ),
    },
]


# Optional BlastGuard-arm steering — appended to the BG arm's system prompt
# when `--bias` is passed. Mirrors bench/prompts.py::BLASTGUARD_BIAS but
# kept self-contained here so microbench has no hidden dependency on the
# broader bench/ prompt module.
BLASTGUARD_BIAS_PROMPT = """

You ALSO have access to the BlastGuard MCP, designed for navigating existing
code. Prefer BlastGuard tools over native alternatives in these situations:

- "What's in this file?" → `blastguard_search` with query 'outline of PATH'.
  Returns every symbol's name + signature + line in 50-300 tokens vs. reading
  the whole file.
- "Who calls this function?" (within the same file) → `blastguard_search` with
  'callers of NAME'. Returns structured caller list. Phase 1 limit: same-file
  only; for cross-file, fall back to grep.
- "Where is this symbol defined?" → `blastguard_search` with 'find NAME'.
  Fuzzy name lookup over the AST graph.
- "What does this file expose publicly?" → `blastguard_search` with
  'exports of PATH'.
- Blast-radius questions — "what breaks if I change X?" — use
  `blastguard_apply_change` to get SIGNATURE / ASYNC_CHANGE / ORPHAN /
  INTERFACE_BREAK cascade warnings + callers/tests context.

Use native tools for reading specific files you already know, cross-file
dependency exploration, writing brand-new files, and ad-hoc bash. Don't
re-grep for a symbol you can ask BlastGuard about.
"""


# ---------------------------------------------------------------------------
# Native tool implementations — what a plain agent has access to.
# ---------------------------------------------------------------------------


def _tool_read(*, path: str, max_bytes: int = 50_000) -> str:
    p = Path(path)
    if not p.is_absolute():
        return f"error: path must be absolute, got {path}"
    if not p.exists():
        return f"error: no such file {path}"
    try:
        data = p.read_text(errors="replace")
    except Exception as e:
        return f"error: {e!r}"
    if len(data) > max_bytes:
        return data[:max_bytes] + f"\n...[truncated, {len(data) - max_bytes} bytes omitted]"
    return data


def _tool_grep(*, pattern: str, path: str, max_matches: int = 50) -> str:
    root = Path(path)
    if not root.is_absolute():
        return f"error: path must be absolute, got {path}"
    try:
        rx = re.compile(pattern)
    except re.error as e:
        return f"error: invalid regex: {e}"
    hits: list[str] = []
    for file in root.rglob("*"):
        if not file.is_file():
            continue
        if any(part.startswith(".") or part in {"target", "node_modules", "__pycache__", ".venv"}
               for part in file.parts):
            continue
        try:
            with file.open("r", errors="replace") as f:
                for lineno, line in enumerate(f, 1):
                    if rx.search(line):
                        hits.append(f"{file}:{lineno}: {line.rstrip()}")
                        if len(hits) >= max_matches:
                            hits.append(f"...[capped at {max_matches} matches]")
                            return "\n".join(hits)
        except (OSError, UnicodeDecodeError):
            continue
    return "\n".join(hits) if hits else "no matches"


def _tool_bash(*, command: str, cwd: str | None = None, timeout: int = 30) -> str:
    try:
        proc = subprocess.run(
            ["bash", "-c", command],
            cwd=cwd,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return f"error: timed out after {timeout}s"
    out = (proc.stdout or "")[-5000:]
    err = (proc.stderr or "")[-2000:]
    return f"exit={proc.returncode}\nstdout:\n{out}\nstderr:\n{err}"


NATIVE_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "read_file",
            "description": "Read an absolute file path. Returns up to 50KB of text.",
            "parameters": {
                "type": "object",
                "properties": {"path": {"type": "string", "description": "Absolute path"}},
                "required": ["path"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "grep",
            "description": "Recursive regex search under a directory. Max 50 matches.",
            "parameters": {
                "type": "object",
                "properties": {
                    "pattern": {"type": "string"},
                    "path": {"type": "string", "description": "Absolute directory root"},
                },
                "required": ["pattern", "path"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "bash",
            "description": "Run a bash command. 30s timeout. Returns exit code + tail of stdout/stderr.",
            "parameters": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "cwd": {"type": "string", "description": "Absolute working directory"},
                },
                "required": ["command"],
            },
        },
    },
]


# ---------------------------------------------------------------------------
# BlastGuard tool wrappers — delegate to the bench/bundles/blastguard/bridge.py.
# ---------------------------------------------------------------------------


def _blastguard_call(*, tool_name: str, json_args: str, project_root: str, binary: str) -> str:
    bridge = Path(__file__).parent / "bundles" / "blastguard" / "bridge.py"
    env = os.environ.copy()
    env["BLASTGUARD_BINARY"] = binary
    env["BLASTGUARD_PROJECT_ROOT"] = project_root
    try:
        proc = subprocess.run(
            [sys.executable, str(bridge), tool_name, json_args],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return "error: blastguard call timed out"
    if proc.returncode != 0:
        return f"blastguard error: {proc.stderr.strip()[:500]}"
    return (proc.stdout or "").strip()[:10_000]


BLASTGUARD_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "blastguard_search",
            "description": (
                "AST-graph query over the project. Pass a single JSON arg with key 'query'. "
                "Verified Phase 1 queries: 'outline of PATH' (all symbols in a file + signatures), "
                "'callers of NAME' (same-file callers), 'find NAME' (fuzzy symbol lookup), "
                "'exports of PATH' (public symbols), 'libraries' (external + internal package list)."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Natural-language structural query"}
                },
                "required": ["query"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "blastguard_apply_change",
            "description": (
                "Apply edits to a file with cascade-warning analysis. JSON args: "
                "{'file': 'path', 'changes': [{'old_text': '...', 'new_text': '...'}]}. "
                "Returns SIGNATURE/ASYNC_CHANGE/ORPHAN/INTERFACE_BREAK warnings + callers/tests context."
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "file": {"type": "string"},
                    "changes": {"type": "array"},
                },
                "required": ["file", "changes"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "blastguard_run_tests",
            "description": (
                "Run the project's test suite (auto-detects pytest/jest/cargo). "
                "JSON args: {'path': 'optional subpath'}. Failures are annotated with "
                "'YOU MODIFIED X' when a stack frame hits a recently-edited symbol."
            ),
            "parameters": {
                "type": "object",
                "properties": {"path": {"type": "string"}},
            },
        },
    },
]


# ---------------------------------------------------------------------------
# Agent loop.
# ---------------------------------------------------------------------------


@dataclass
class RunResult:
    task_id: str
    arm: str
    turns: int
    input_tokens: int
    cached_input_tokens: int
    output_tokens: int
    wall_seconds: float
    tool_calls: dict[str, int]
    final_answer: str
    stopped_reason: str
    input_cost_usd: float
    output_cost_usd: float
    total_cost_usd: float


def _execute_tool(name: str, args: dict[str, Any], *, project_root: str, binary: str) -> str:
    if name == "read_file":
        return _tool_read(**args)
    if name == "grep":
        return _tool_grep(**args)
    if name == "bash":
        return _tool_bash(cwd=args.get("cwd", project_root), **{k: v for k, v in args.items() if k != "cwd"})
    if name == "blastguard_search":
        return _blastguard_call(
            tool_name="search",
            json_args=json.dumps({"query": args.get("query", "")}),
            project_root=project_root,
            binary=binary,
        )
    if name == "blastguard_apply_change":
        return _blastguard_call(
            tool_name="apply_change",
            json_args=json.dumps(args),
            project_root=project_root,
            binary=binary,
        )
    if name == "blastguard_run_tests":
        return _blastguard_call(
            tool_name="run_tests",
            json_args=json.dumps(args),
            project_root=project_root,
            binary=binary,
        )
    return f"error: unknown tool {name!r}"


def run_task(
    *,
    task: dict[str, str],
    arm: str,
    model: str,
    project_root: str,
    blastguard_binary: str,
    max_turns: int = 25,
    in_price: float = 0.30,
    out_price: float = 1.20,
    apply_bias: bool = True,
) -> RunResult:
    from openai import OpenAI  # noqa: PLC0415

    tools = NATIVE_TOOLS[:]
    if arm == "blastguard":
        tools += BLASTGUARD_TOOLS

    client = OpenAI(
        api_key=os.environ["OPENROUTER_API_KEY"],
        base_url="https://openrouter.ai/api/v1",
    )

    system_prompt = (
        "You are a software engineer exploring a codebase to answer a question. "
        "Use the available tools to read files, search, and reason. When you "
        "have enough information, give a concise answer in plain text and "
        "write 'DONE' on its own line to stop."
    )
    if arm == "blastguard" and apply_bias:
        system_prompt += BLASTGUARD_BIAS_PROMPT
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system_prompt},
        {"role": "user", "content": task["prompt"].format(project_root=project_root)},
    ]

    total_in = 0
    total_cached = 0
    total_out = 0
    tool_counts: dict[str, int] = {}
    final_answer = ""
    stopped_reason = "max_turns"
    t0 = time.time()
    turn = 0

    for turn in range(max_turns):  # noqa: B007 — used for turn-count in result
        resp = client.chat.completions.create(
            model=model,
            messages=messages,
            tools=tools,
            max_tokens=4096,
        )
        usage = resp.usage
        total_in += getattr(usage, "prompt_tokens", 0) or 0
        total_out += getattr(usage, "completion_tokens", 0) or 0
        if usage and hasattr(usage, "prompt_tokens_details") and usage.prompt_tokens_details:
            total_cached += getattr(usage.prompt_tokens_details, "cached_tokens", 0) or 0

        choice = resp.choices[0]
        msg = choice.message
        content_text = msg.content or ""

        assistant_entry: dict[str, Any] = {
            "role": "assistant",
            "content": content_text,
        }
        if msg.tool_calls:
            assistant_entry["tool_calls"] = [
                {
                    "id": tc.id,
                    "type": "function",
                    "function": {"name": tc.function.name, "arguments": tc.function.arguments},
                }
                for tc in msg.tool_calls
            ]
        messages.append(assistant_entry)

        if msg.tool_calls:
            for tc in msg.tool_calls:
                name = tc.function.name
                tool_counts[name] = tool_counts.get(name, 0) + 1
                try:
                    args = json.loads(tc.function.arguments or "{}")
                except json.JSONDecodeError:
                    args = {}
                result = _execute_tool(
                    name, args, project_root=project_root, binary=blastguard_binary
                )
                messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": result,
                    }
                )
            continue  # let the model react to the tool results

        # No tool calls — this is a terminal or near-terminal turn.
        if "DONE" in content_text.split():
            final_answer = content_text
            stopped_reason = "done_marker"
            break
        if choice.finish_reason == "stop":
            final_answer = content_text
            stopped_reason = "finish_stop"
            break
        final_answer = content_text

    wall = time.time() - t0
    input_cost = total_in * in_price / 1_000_000.0
    output_cost = total_out * out_price / 1_000_000.0

    return RunResult(
        task_id=task["id"],
        arm=arm,
        turns=turn + 1,
        input_tokens=total_in,
        cached_input_tokens=total_cached,
        output_tokens=total_out,
        wall_seconds=wall,
        tool_calls=tool_counts,
        final_answer=final_answer,
        stopped_reason=stopped_reason,
        input_cost_usd=input_cost,
        output_cost_usd=output_cost,
        total_cost_usd=input_cost + output_cost,
    )


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--model", default="minimax/minimax-m2.7")
    p.add_argument(
        "--project-root",
        default=str(Path(__file__).resolve().parent.parent),
        help="Absolute path to the repo to explore (default: BlastGuard itself)",
    )
    p.add_argument(
        "--blastguard-binary",
        default=str(Path(__file__).resolve().parent.parent / "target" / "release" / "blastguard"),
    )
    p.add_argument("--max-turns", type=int, default=25)
    p.add_argument("--output", type=Path, default=Path(__file__).parent / "results" / "microbench.jsonl")
    args = p.parse_args()

    if "OPENROUTER_API_KEY" not in os.environ:
        print("ERROR: OPENROUTER_API_KEY not set", file=sys.stderr)
        return 2

    args.output.parent.mkdir(parents=True, exist_ok=True)
    results: list[RunResult] = []

    for task in TASKS:
        for arm in ("raw", "blastguard"):
            print(f"\n=== task={task['id']} arm={arm} ===")
            r = run_task(
                task=task,
                arm=arm,
                model=args.model,
                project_root=args.project_root,
                blastguard_binary=args.blastguard_binary,
                max_turns=args.max_turns,
            )
            print(
                f"  turns={r.turns} in={r.input_tokens} out={r.output_tokens} "
                f"cost=${r.total_cost_usd:.4f} wall={r.wall_seconds:.1f}s "
                f"stop={r.stopped_reason}"
            )
            print(f"  tools: {r.tool_calls}")
            print(f"  answer (first 200 chars): {r.final_answer[:200]!r}")
            results.append(r)

    with args.output.open("w", encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(asdict(r)) + "\n")

    # Summary table
    print("\n\n=== SUMMARY ===")
    print(f"{'task':<24} {'arm':<12} {'turns':>5} {'in_tok':>8} {'out_tok':>7} {'cost':>8} {'wall':>6}")
    for r in results:
        print(
            f"{r.task_id:<24} {r.arm:<12} {r.turns:>5} {r.input_tokens:>8} "
            f"{r.output_tokens:>7} ${r.total_cost_usd:>6.4f} {r.wall_seconds:>5.1f}s"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
