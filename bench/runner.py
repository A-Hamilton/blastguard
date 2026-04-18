"""Top-level benchmark runner.

```
uv run python runner.py --tasks 3 --model claude-opus-4-7 --no-blastguard
uv run python runner.py --tasks 3 --model claude-opus-4-7 --with-blastguard
```

Each task:
1. Clone the repo at base_commit into a tempdir workspace.
2. Start BlastGuard subprocess (if --with-blastguard) against the workspace.
3. Run the agent loop with the problem_statement as the user message.
4. Collect the list of files the agent modified.
5. Run the grader.
6. Emit a JSONL record to `results/<run_name>.jsonl`.
"""

from __future__ import annotations

import asyncio
import json
import subprocess
import time
from pathlib import Path
from typing import Any

import click

from bench.agent_loop import LoopResult, run_anthropic, run_openai_compatible
from bench.grader import grade
from bench.mcp_client import blastguard_session, find_blastguard_binary
from bench.prompts import BASELINE_SYSTEM, BLASTGUARD_SYSTEM
from bench.tasks import Task, load_tasks

REPO_ROOT = Path(__file__).resolve().parent.parent


def setup_workspace(task: Task, root: Path) -> Path:
    """Clone the repo at the task's base_commit into `root / task_id`."""
    workspace = root / task.task_id
    workspace.mkdir(parents=True, exist_ok=False)
    subprocess.run(
        ["git", "clone", f"https://github.com/{task.repo}.git", str(workspace)],
        check=True,
        capture_output=True,
    )
    subprocess.run(
        ["git", "-C", str(workspace), "checkout", task.base_commit],
        check=True,
        capture_output=True,
    )
    return workspace


def changed_files(workspace: Path) -> list[str]:
    """Paths modified relative to HEAD."""
    result = subprocess.run(
        ["git", "-C", str(workspace), "diff", "--name-only", "HEAD"],
        capture_output=True,
        text=True,
        check=False,
    )
    return [line for line in result.stdout.splitlines() if line.strip()]


async def run_one_task(
    task: Task,
    workspace_root: Path,
    model: str,
    provider: str,
    with_blastguard: bool,
) -> dict[str, Any]:
    """Run a single task end-to-end. Returns the JSONL record dict."""
    started = time.time()
    workspace = setup_workspace(task, workspace_root)
    system_prompt = BLASTGUARD_SYSTEM if with_blastguard else BASELINE_SYSTEM

    # Minimal native tool schemas for the baseline scaffold. For Phase 1 we
    # keep these stubbed — the agent can call them but the executor below
    # only forwards to BlastGuard when with_blastguard=True. A real
    # baseline would implement bash/editor here; that's Phase 2 work.
    native_tools: list[dict[str, Any]] = [
        {
            "name": "bash",
            "description": "Run a shell command in the workspace.",
            "input_schema": {
                "type": "object",
                "properties": {"command": {"type": "string"}},
                "required": ["command"],
            },
        },
        {
            "name": "str_replace_editor",
            "description": "Edit a file via str_replace or create.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "path": {"type": "string"},
                    "old_str": {"type": "string"},
                    "new_str": {"type": "string"},
                    "file_text": {"type": "string"},
                },
                "required": ["command", "path"],
            },
        },
    ]

    async def _exec_native(name: str, args: dict[str, Any]) -> str:
        if name == "bash":
            cmd = args.get("command", "")
            res = subprocess.run(
                cmd,
                shell=True,
                cwd=workspace,
                capture_output=True,
                text=True,
                timeout=60,
            )
            return (res.stdout + res.stderr)[:4000]
        if name == "str_replace_editor":
            cmd = args.get("command", "")
            path = workspace / args.get("path", "")
            if cmd == "create":
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(args.get("file_text", ""))
                return f"Created {path}"
            if cmd == "str_replace":
                src = path.read_text()
                src = src.replace(args["old_str"], args["new_str"], 1)
                path.write_text(src)
                return f"Edited {path}"
            if cmd == "view":
                return path.read_text()[:4000]
        return f"unknown tool: {name}"

    loop_result: LoopResult
    if with_blastguard:
        binary = find_blastguard_binary(REPO_ROOT)
        async with blastguard_session(workspace, binary) as mcp_session:
            tools_list = await mcp_session.list_tools()
            mcp_tool_schemas = [
                {
                    "name": t.name,
                    "description": t.description or "",
                    "input_schema": t.inputSchema or {"type": "object"},
                }
                for t in tools_list.tools
            ]
            all_tools = native_tools + mcp_tool_schemas
            bg_tool_names = {t.name for t in tools_list.tools}

            async def _exec(name: str, args: dict[str, Any]) -> str:
                if name in bg_tool_names:
                    call_result = await mcp_session.call_tool(name, args)
                    if call_result.isError:
                        first = call_result.content[0].text if call_result.content else ""
                        return f"[BlastGuard error] {first}"
                    return "\n".join(
                        getattr(c, "text", "") for c in call_result.content
                    )[:4000]
                return await _exec_native(name, args)

            loop_result = await _dispatch_agent(
                provider=provider,
                model=model,
                system=system_prompt,
                user_message=task.problem_statement,
                tools=all_tools,
                executor=_exec,
            )
    else:
        loop_result = await _dispatch_agent(
            provider=provider,
            model=model,
            system=system_prompt,
            user_message=task.problem_statement,
            tools=native_tools,
            executor=_exec_native,
        )

    mutated = changed_files(workspace)
    grade_result = grade(
        workspace=workspace,
        changed_files=mutated,
        fail_to_pass=task.fail_to_pass,
        pass_to_pass=task.pass_to_pass,
    )

    return {
        "task_id": task.task_id,
        "repo": task.repo,
        "model": model,
        "with_blastguard": with_blastguard,
        "resolved": grade_result.resolved,
        "tampered": grade_result.tampered,
        "tampered_files": list(grade_result.tampered_files),
        "turns": len(loop_result.turns),
        "tokens_in": loop_result.total_tokens_in,
        "tokens_out": loop_result.total_tokens_out,
        "tool_calls_per_type": loop_result.tool_calls_per_type(),
        "fail_to_pass_passed": grade_result.fail_to_pass_passed,
        "fail_to_pass_total": grade_result.fail_to_pass_total,
        "pass_to_pass_passed": grade_result.pass_to_pass_passed,
        "pass_to_pass_total": grade_result.pass_to_pass_total,
        "wall_time_s": round(time.time() - started, 1),
    }


async def _dispatch_agent(
    *,
    provider: str,
    model: str,
    system: str,
    user_message: str,
    tools: list[dict[str, Any]],
    executor,
) -> LoopResult:
    if provider == "anthropic":
        return await run_anthropic(model, system, user_message, tools, executor)
    if provider == "openai":
        return await run_openai_compatible(model, system, user_message, tools, executor)
    raise ValueError(f"unknown provider: {provider}")


@click.command()
@click.option("--tasks", type=int, default=3, help="Number of tasks to run")
@click.option("--model", required=True, help="Model name (e.g., claude-opus-4-7, glm-5.1)")
@click.option("--provider", type=click.Choice(["anthropic", "openai"]), default="anthropic")
@click.option("--with-blastguard/--no-blastguard", default=False)
@click.option("--output", type=click.Path(path_type=Path), default=None)
def main(
    tasks: int,
    model: str,
    provider: str,
    with_blastguard: bool,
    output: Path | None,
) -> None:
    """Run the benchmark harness."""
    out_name = output or (
        Path("results")
        / f"{'blastguard' if with_blastguard else 'baseline'}-{model}-{tasks}tasks.jsonl"
    )
    out_name.parent.mkdir(exist_ok=True)
    tasks_list = load_tasks(limit=tasks)
    workspace_root = Path("/tmp") / f"blastguard-bench-{int(time.time())}"
    workspace_root.mkdir()

    with out_name.open("w", encoding="utf-8") as f:
        for task in tasks_list:
            try:
                record = asyncio.run(
                    run_one_task(
                        task=task,
                        workspace_root=workspace_root,
                        model=model,
                        provider=provider,
                        with_blastguard=with_blastguard,
                    )
                )
            except Exception as e:  # noqa: BLE001
                record = {
                    "task_id": task.task_id,
                    "error": repr(e),
                    "resolved": False,
                }
            f.write(json.dumps(record) + "\n")
            f.flush()
            click.echo(f"[{task.task_id}] resolved={record.get('resolved')}")

    # Keep the workspaces around for debugging. Prune manually later.
    click.echo(f"\nResults: {out_name}")
    click.echo(f"Workspaces: {workspace_root}")


if __name__ == "__main__":
    main()
