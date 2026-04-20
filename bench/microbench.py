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
# Tasks — imported from the centralized registry.
# ---------------------------------------------------------------------------

from bench.tasks_registry import TASKS  # noqa: E402


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

IMPORTANT — EFFICIENCY RULES:

1. ONE TOOL PER QUESTION. If `blastguard_search 'outline of X'` already
   shows the function you care about with its signature and line number,
   that IS the answer — do NOT additionally `read_file` on the same path
   to "confirm". The outline is authoritative.
2. DON'T STACK TOOLS. Never call `blastguard_search` AND `read_file` AND
   `grep` on the same target in one task unless each returned something
   genuinely new. Pick the most specific tool first, then stop.
3. ANSWER AS SOON AS YOU HAVE ENOUGH. The goal is a correct short answer
   in minimum turns. If you already have the file:line and the signature,
   you're done — write the answer and DONE.
4. Every extra turn costs tokens on ALL prior context. A 4-turn solve
   is ~50% cheaper than a 6-turn solve. Aim for fewest turns.

STEP-TYPE CLASSIFICATION (before EVERY tool call):

Classify each step as reflexive or deliberative:
- REFLEXIVE: the answer is already in the conversation context — a prior
  tool result already contains what you need. DO NOT call a tool. Write
  the answer directly.
- DELIBERATIVE: you need information you genuinely do not have yet. Call
  a tool.

Before any tool call, state `step: deliberative — need X because Y` in
one short line. If the step is reflexive, skip the tool call entirely
and go straight to the answer. This classification is mandatory — it is
the single biggest defense against redundant tool chains.
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


# Tool descriptions are deliberately minimal — per arXiv:2602.14878, verbose
# examples and overlapping parameter-explanation prose can increase execution
# steps ~67% without improving accuracy. Keep purpose + supported values only.
BLASTGUARD_TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "blastguard_search",
            "description": (
                "Query AST code graph. query values: 'outline of PATH', 'find NAME', "
                "'callers of NAME', 'exports of PATH', 'libraries'."
            ),
            "parameters": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"],
            },
        },
    },
    {
        "type": "function",
        "function": {
            "name": "blastguard_apply_change",
            "description": "Apply edits to a file with cascade warnings.",
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
            "description": "Run tests (auto-detects pytest/jest/cargo).",
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
    seed: int
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
    in_price: float = 0.30,  # USD per M input tokens; set to 0.0 for local models
    out_price: float = 1.20,
    apply_bias: bool = True,
    api_base: str = "https://openrouter.ai/api/v1",
    api_key_env: str = "OPENROUTER_API_KEY",
    model_id_for_api: str | None = None,  # if set, use this in the request instead of `model`
    seed_value: int = 1,  # reproducibility marker in the output record
) -> RunResult:
    from openai import OpenAI  # noqa: PLC0415

    if arm == "blastguard":
        # On the BG arm, hide `read_file` and `grep` entirely — their use cases
        # (read a file / search for a string) are covered by `blastguard_search`
        # outline / find queries. Keep `bash` as a fallback so the model can
        # still run ad-hoc commands (cat, find, rg) when BlastGuard doesn't
        # have what it needs (e.g. cross-file dependencies, Phase 2 territory).
        #
        # This is a deliberate departure from the "bias, don't force" principle
        # in CodeCompass — we tried bias via prompt language and the model
        # stacked tools redundantly on easy tasks. Removing the redundant
        # natives is a hard constraint that forces the tool-choice pattern the
        # prompt was asking for.
        tools = [t for t in NATIVE_TOOLS if t["function"]["name"] == "bash"]
        tools += BLASTGUARD_TOOLS
    else:
        tools = NATIVE_TOOLS[:]

    # 10-minute per-request timeout + 5 SDK-level retries. Local Gemma can
    # stall briefly on big prompts; we'd rather wait than crash the whole run.
    client = OpenAI(
        api_key=os.environ.get(api_key_env, "not-needed-for-local"),
        base_url=api_base,
        max_retries=5,
        timeout=600.0,
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
            model=model_id_for_api if model_id_for_api is not None else model,
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
        seed=seed_value,
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
    p.add_argument("--in-price", type=float, default=0.30, dest="in_price")
    p.add_argument("--out-price", type=float, default=1.20, dest="out_price")
    p.add_argument("--cache-price", type=float, default=0.0, dest="cache_price")
    p.add_argument(
        "--api-base",
        default="https://openrouter.ai/api/v1",
        help="OpenAI-compatible API base URL (set to http://127.0.0.1:8080/v1 for local Gemma)",
    )
    p.add_argument(
        "--api-key-env",
        default="OPENROUTER_API_KEY",
        help="Env var name to read the API key from. Local servers usually accept any value; still required to be set.",
    )
    p.add_argument(
        "--seeds",
        type=int,
        default=1,
        help="Run each (task, arm) pair this many times with seeds 1..N. "
             "Extra seeds give us variance estimates for stats_aggregate.py.",
    )
    p.add_argument(
        "--model-id-override",
        default=None,
        help="Override the model ID sent in the chat/completions request while keeping "
             "the --model value in the output log. Use when the local endpoint expects "
             "a short ID (e.g. 'gemma-4') but you want the log tagged with the full name.",
    )
    p.add_argument(
        "--tasks",
        default=None,
        help="Comma-separated list of task IDs to run (e.g. 'chain-search-to-graph'). "
             "Defaults to all tasks in tasks_registry.py.",
    )
    args = p.parse_args()

    if args.tasks is None:
        selected_tasks = TASKS
    else:
        wanted = {t.strip() for t in args.tasks.split(",") if t.strip()}
        selected_tasks = [t for t in TASKS if t["id"] in wanted]
        missing = wanted - {t["id"] for t in selected_tasks}
        if missing:
            print(f"ERROR: unknown task id(s): {sorted(missing)}", file=sys.stderr)
            return 2
        if not selected_tasks:
            print("ERROR: --tasks matched no tasks", file=sys.stderr)
            return 2

    if args.api_key_env not in os.environ:
        print(f"ERROR: {args.api_key_env} not set", file=sys.stderr)
        return 2

    args.output.parent.mkdir(parents=True, exist_ok=True)
    results: list[RunResult] = []

    for seed_idx in range(1, args.seeds + 1):
        for task in selected_tasks:
            for arm in ("raw", "blastguard"):
                print(f"\n=== task={task['id']} arm={arm} seed={seed_idx} ===")
                r = run_task(
                    task=task,
                    arm=arm,
                    model=args.model,
                    project_root=args.project_root,
                    blastguard_binary=args.blastguard_binary,
                    max_turns=args.max_turns,
                    in_price=args.in_price,
                    out_price=args.out_price,
                    api_base=args.api_base,
                    api_key_env=args.api_key_env,
                    model_id_for_api=args.model_id_override,
                    seed_value=seed_idx,
                )
                print(
                    f"  seed={seed_idx} turns={r.turns} in={r.input_tokens} "
                    f"out={r.output_tokens} cost=${r.total_cost_usd:.4f} wall={r.wall_seconds:.1f}s "
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
