"""Model-agnostic agent loop with tool-use.

The loop is deliberately simple for Phase 1:
- One system prompt, one user turn (the task's problem_statement).
- The agent calls tools; we execute them and feed results back.
- Cap at `max_turns` (default 50).
- Emit per-turn instrumentation (tokens in/out, tool calls).

Providers supported:
- Anthropic: model like `claude-opus-4-7`, `claude-sonnet-4-6`.
- OpenAI-compatible: model like `glm-5.1` via OpenRouter (OPENROUTER_API_KEY).
"""

from __future__ import annotations

import os
import time
from collections.abc import Awaitable, Callable
from dataclasses import dataclass, field
from typing import Any


@dataclass
class TurnRecord:
    turn_index: int
    tokens_in: int
    tokens_out: int
    tool_calls: list[str]
    duration_ms: int


@dataclass
class LoopResult:
    turns: list[TurnRecord] = field(default_factory=list)
    finished_cleanly: bool = False
    final_text: str = ""
    total_tokens_in: int = 0
    total_tokens_out: int = 0

    def tool_calls_per_type(self) -> dict[str, int]:
        counts: dict[str, int] = {}
        for t in self.turns:
            for name in t.tool_calls:
                counts[name] = counts.get(name, 0) + 1
        return counts


ToolExecutor = Callable[[str, dict[str, Any]], Awaitable[str]]


async def run_anthropic(
    model: str,
    system: str,
    user_message: str,
    tool_schemas: list[dict[str, Any]],
    tool_executor: ToolExecutor,
    max_turns: int = 50,
) -> LoopResult:
    """Agent loop against the Anthropic API."""
    import anthropic
    client = anthropic.AsyncAnthropic(api_key=os.environ["ANTHROPIC_API_KEY"])
    result = LoopResult()
    messages: list[dict[str, Any]] = [{"role": "user", "content": user_message}]

    for turn_index in range(max_turns):
        t0 = time.monotonic()
        response = await client.messages.create(
            model=model,
            max_tokens=4096,
            system=system,
            tools=tool_schemas,
            messages=messages,
        )
        duration_ms = int((time.monotonic() - t0) * 1000)
        tokens_in = response.usage.input_tokens
        tokens_out = response.usage.output_tokens
        result.total_tokens_in += tokens_in
        result.total_tokens_out += tokens_out

        tool_calls_this_turn: list[str] = []
        tool_results: list[dict[str, Any]] = []
        assistant_content: list[dict[str, Any]] = []
        stop_on_done = False

        for block in response.content:
            block_type = getattr(block, "type", None)
            if block_type == "text":
                text = getattr(block, "text", "")
                assistant_content.append({"type": "text", "text": text})
                if "DONE" in text.strip().upper().split():
                    stop_on_done = True
                    result.final_text = text
            elif block_type == "tool_use":
                name = block.name
                args = dict(block.input) if hasattr(block, "input") else {}
                tool_calls_this_turn.append(name)
                assistant_content.append(
                    {
                        "type": "tool_use",
                        "id": block.id,
                        "name": name,
                        "input": args,
                    }
                )
                try:
                    output = await tool_executor(name, args)
                except Exception as e:  # noqa: BLE001
                    output = f"tool error: {e!r}"
                tool_results.append(
                    {
                        "type": "tool_result",
                        "tool_use_id": block.id,
                        "content": output,
                    }
                )

        result.turns.append(
            TurnRecord(
                turn_index=turn_index,
                tokens_in=tokens_in,
                tokens_out=tokens_out,
                tool_calls=tool_calls_this_turn,
                duration_ms=duration_ms,
            )
        )

        messages.append({"role": "assistant", "content": assistant_content})
        if tool_results:
            messages.append({"role": "user", "content": tool_results})

        if stop_on_done and not tool_calls_this_turn:
            result.finished_cleanly = True
            break
        if response.stop_reason == "end_turn" and not tool_calls_this_turn:
            break

    return result


async def run_openai_compatible(
    model: str,
    system: str,
    user_message: str,
    tool_schemas: list[dict[str, Any]],
    tool_executor: ToolExecutor,
    max_turns: int = 50,
    api_key_env: str = "OPENROUTER_API_KEY",
    base_url: str = "https://openrouter.ai/api/v1",
) -> LoopResult:
    """Agent loop against an OpenAI-compatible endpoint (for GLM, etc.)."""
    import openai
    client = openai.AsyncOpenAI(
        api_key=os.environ[api_key_env],
        base_url=base_url,
    )
    # OpenAI tool_use schema shape differs from Anthropic's. Convert.
    openai_tools = [
        {
            "type": "function",
            "function": {
                "name": t["name"],
                "description": t.get("description", ""),
                "parameters": t.get("input_schema", {"type": "object"}),
            },
        }
        for t in tool_schemas
    ]
    result = LoopResult()
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system},
        {"role": "user", "content": user_message},
    ]

    for turn_index in range(max_turns):
        t0 = time.monotonic()
        response = await client.chat.completions.create(
            model=model,
            messages=messages,
            tools=openai_tools,
            max_tokens=4096,
        )
        duration_ms = int((time.monotonic() - t0) * 1000)
        usage = response.usage
        tokens_in = getattr(usage, "prompt_tokens", 0) or 0
        tokens_out = getattr(usage, "completion_tokens", 0) or 0
        result.total_tokens_in += tokens_in
        result.total_tokens_out += tokens_out

        choice = response.choices[0]
        msg = choice.message
        tool_calls_this_turn: list[str] = []
        assistant_payload: dict[str, Any] = {
            "role": "assistant",
            "content": msg.content or "",
        }
        if msg.tool_calls:
            assistant_payload["tool_calls"] = [
                {
                    "id": tc.id,
                    "type": "function",
                    "function": {"name": tc.function.name, "arguments": tc.function.arguments},
                }
                for tc in msg.tool_calls
            ]
        messages.append(assistant_payload)

        if msg.tool_calls:
            import json as _json
            for tc in msg.tool_calls:
                tool_calls_this_turn.append(tc.function.name)
                try:
                    args = _json.loads(tc.function.arguments or "{}")
                except _json.JSONDecodeError:
                    args = {}
                try:
                    output = await tool_executor(tc.function.name, args)
                except Exception as e:  # noqa: BLE001
                    output = f"tool error: {e!r}"
                messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": output,
                    }
                )

        result.turns.append(
            TurnRecord(
                turn_index=turn_index,
                tokens_in=tokens_in,
                tokens_out=tokens_out,
                tool_calls=tool_calls_this_turn,
                duration_ms=duration_ms,
            )
        )

        if "DONE" in (msg.content or "").upper().split() and not tool_calls_this_turn:
            result.final_text = msg.content or ""
            result.finished_cleanly = True
            break
        if choice.finish_reason == "stop" and not tool_calls_this_turn:
            break

    return result
