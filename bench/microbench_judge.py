"""LLM-as-judge microbench grader — Priority 1b quality measurement.

Pairs up same-(task, seed) rollouts from the raw and blastguard arms
and asks a second LLM instance (by default the same local Gemma the
benchmark uses) to pick the better answer blind.

Catches quality dimensions the deterministic substring grader in
`microbench_grader.py` misses:

- Fluency — both answers might mention the required substrings, but
  one is incoherent or buried in noise.
- Hallucination — substring-correct but contains fabricated details.
- Substance — technically answers the question but misses the point,
  or goes in circles.

**Bias caveat.** Gemma judging Gemma's output introduces self-preference
bias (well-documented in the LLM-as-judge literature, e.g.
arXiv:2306.05685). Mitigations built in:

1. **Blind randomized A/B labelling.** Each judge call randomly maps
   arms to letters A and B; the judge never sees the arm names. An
   arm that wins consistently must win across both letter
   assignments, not just preference for "A" or "B".
2. **Multiple judge samples.** `n_judges=3` default: three independent
   judge calls per pair, majority-vote the winner. Reduces flakiness
   on borderline pairs.
3. **Scored rubric, not just a pick.** Judge returns a JSON object
   with picks for three axes (correctness, substance, conciseness)
   plus a brief reason. Makes the judgment auditable.

**Not implemented yet (documented here for the next iteration):**

- Cross-model judging (e.g. a cloud Sonnet judging local Gemma). Would
  break self-preference bias but requires a paid API, which the
  project rules out.
- Positional-order permutation beyond A/B randomization (the judge can
  still show a "first position" bias within a single call). Rotating
  the order between judge runs is a next step.

Library-only; callers wire in an OpenAI-compatible client (the
microbench's own client works fine). Returns structured verdicts
so the `bench-regression-guard` subagent can consume them.
"""

from __future__ import annotations

import json
import random
import re
from dataclasses import asdict, dataclass
from typing import Any, Literal, Protocol

JudgePick = Literal["raw", "blastguard", "tie"]


class ChatClient(Protocol):
    """Minimal OpenAI-compatible chat client interface for the judge."""

    def create(self, *, model: str, messages: list[dict[str, str]], temperature: float) -> Any:
        ...


@dataclass
class AxisJudgment:
    """Judgment on one rubric axis."""

    pick: JudgePick
    reason: str


@dataclass
class JudgeVerdict:
    """Aggregated verdict across `n_judges` runs for a single (task, seed) pair."""

    task_id: str
    seed: int
    winner: JudgePick
    n_judges: int
    raw_wins: int
    bg_wins: int
    ties: int
    axis_summary: dict[str, JudgePick]  # "correctness" | "substance" | "conciseness"
    raw_answer: str
    blastguard_answer: str
    per_judge: list[dict[str, Any]]


_JUDGE_SYSTEM = """\
You are a strict quality judge comparing two answers to the same
question. Pick the better answer on three independent axes:

- correctness: does the answer accurately and completely address the
  question? Penalise hallucinations and wrong file/function names.
- substance: does the answer demonstrate understanding, not just
  surface-level pattern matching? Prefer specificity over hand-waving.
- conciseness: does the answer avoid padding, preamble, and
  unnecessary elaboration? Shorter is better when correctness and
  substance tie.

For each axis, pick "A", "B", or "tie". Also give a one-sentence
reason per axis. If an answer is malformed (e.g. not actual prose,
truncated at a tool-call, or echoes the question), mark the other
one better on every axis.

Respond ONLY with a JSON object of the shape:

{
  "correctness": {"pick": "A" | "B" | "tie", "reason": "..."},
  "substance":   {"pick": "A" | "B" | "tie", "reason": "..."},
  "conciseness": {"pick": "A" | "B" | "tie", "reason": "..."}
}

No prose outside the JSON. No markdown code fence. No "thinking aloud".
"""


def _build_judge_user_message(task_prompt: str, answer_a: str, answer_b: str) -> str:
    # Trim answers to stay inside a reasonable judge-context budget.
    # Long tool-trace transcripts dominate the signal anyway; the final
    # prose answer is what we grade on.
    def trim(s: str, max_chars: int = 4000) -> str:
        s = s.strip()
        return s if len(s) <= max_chars else s[:max_chars] + "\n[…truncated]"

    return (
        f"Question:\n{trim(task_prompt, max_chars=2000)}\n\n"
        f"--- Answer A ---\n{trim(answer_a)}\n\n"
        f"--- Answer B ---\n{trim(answer_b)}\n"
    )


_JSON_OBJECT_RE = re.compile(r"\{.*\}", re.DOTALL)


def _parse_judge_response(text: str) -> dict[str, dict[str, str]] | None:
    """Extract the JSON object from the judge's response.

    The judge occasionally emits a short preamble even when told not to.
    Fall back to grabbing the first balanced `{...}` block.
    """
    stripped = text.strip()
    # Fast path: whole response is the JSON object.
    try:
        parsed = json.loads(stripped)
        if isinstance(parsed, dict):
            return parsed
    except json.JSONDecodeError:
        pass
    match = _JSON_OBJECT_RE.search(stripped)
    if match is None:
        return None
    try:
        parsed = json.loads(match.group(0))
    except json.JSONDecodeError:
        return None
    return parsed if isinstance(parsed, dict) else None


def _score_one_judge(
    parsed: dict[str, dict[str, str]],
    a_is_raw: bool,
) -> dict[str, AxisJudgment]:
    """Map judge's A/B picks back to raw/blastguard."""

    def translate(letter: str) -> JudgePick:
        upper = letter.strip().upper()
        if upper == "A":
            return "raw" if a_is_raw else "blastguard"
        if upper == "B":
            return "blastguard" if a_is_raw else "raw"
        return "tie"

    result: dict[str, AxisJudgment] = {}
    for axis in ("correctness", "substance", "conciseness"):
        entry = parsed.get(axis)
        # Defensive: Gemma sometimes returns a bare string (e.g. "A") or null
        # instead of the expected {"pick": "...", "reason": "..."} dict. Coerce
        # both shapes into our internal record without raising.
        if isinstance(entry, dict):
            pick = translate(str(entry.get("pick", "tie")))
            reason = str(entry.get("reason", "")).strip()
        elif isinstance(entry, str):
            pick = translate(entry)
            reason = ""
        else:
            pick = "tie"
            reason = ""
        result[axis] = AxisJudgment(pick=pick, reason=reason)
    return result


def _majority(picks: list[JudgePick]) -> JudgePick:
    """Plurality with tie-goes-to-tie fallback on ≥2-way split."""
    tally: dict[JudgePick, int] = {"raw": 0, "blastguard": 0, "tie": 0}
    for p in picks:
        tally[p] = tally.get(p, 0) + 1
    top = max(tally.values())
    winners = [p for p, n in tally.items() if n == top]
    return winners[0] if len(winners) == 1 else "tie"


def judge_pair(
    *,
    task_id: str,
    task_prompt: str,
    raw_answer: str,
    blastguard_answer: str,
    seed: int,
    client: ChatClient,
    model: str,
    n_judges: int = 3,
    rng_seed: int = 17,
) -> JudgeVerdict:
    """Run `n_judges` independent judge calls on a single (task, seed) pair.

    Each call randomly swaps which arm is labelled A vs B. Per-axis
    picks are translated back to arm names and aggregated.

    The overall `winner` is the majority pick on the `correctness`
    axis; substance and conciseness are surfaced for auditability but
    do not override correctness. If all three correctness picks are
    different (one raw, one bg, one tie) the verdict is `tie`.
    """
    rng = random.Random(rng_seed + seed)
    per_judge: list[dict[str, Any]] = []
    axis_tallies: dict[str, list[JudgePick]] = {
        "correctness": [],
        "substance": [],
        "conciseness": [],
    }
    raw_wins = bg_wins = ties = 0
    for judge_idx in range(n_judges):
        a_is_raw = rng.random() < 0.5
        answer_a = raw_answer if a_is_raw else blastguard_answer
        answer_b = blastguard_answer if a_is_raw else raw_answer

        messages = [
            {"role": "system", "content": _JUDGE_SYSTEM},
            {"role": "user", "content": _build_judge_user_message(task_prompt, answer_a, answer_b)},
        ]
        response = client.create(
            model=model,
            messages=messages,
            temperature=0.2,
        )
        text = response.choices[0].message.content or ""
        parsed = _parse_judge_response(text)
        if parsed is None:
            per_judge.append({
                "judge_idx": judge_idx,
                "a_is_raw": a_is_raw,
                "parse_error": True,
                "raw_text": text[:500],
            })
            continue

        scored = _score_one_judge(parsed, a_is_raw)
        for axis, judgment in scored.items():
            axis_tallies[axis].append(judgment.pick)
        per_judge.append({
            "judge_idx": judge_idx,
            "a_is_raw": a_is_raw,
            "scored": {axis: asdict(j) for axis, j in scored.items()},
        })
        correctness_pick = scored["correctness"].pick
        if correctness_pick == "raw":
            raw_wins += 1
        elif correctness_pick == "blastguard":
            bg_wins += 1
        else:
            ties += 1

    axis_summary = {axis: _majority(picks) for axis, picks in axis_tallies.items()}
    winner = axis_summary.get("correctness", "tie")

    return JudgeVerdict(
        task_id=task_id,
        seed=seed,
        winner=winner,
        n_judges=n_judges,
        raw_wins=raw_wins,
        bg_wins=bg_wins,
        ties=ties,
        axis_summary=axis_summary,
        raw_answer=raw_answer,
        blastguard_answer=blastguard_answer,
        per_judge=per_judge,
    )


def aggregate_verdicts(verdicts: list[JudgeVerdict]) -> dict[str, dict[str, Any]]:
    """Summarise per-task correctness win rates across all judged pairs.

    Returns `{task_id: {"n": int, "raw_wins": int, "bg_wins": int,
    "ties": int, "bg_win_rate": float}}`. Useful companion output to
    `bench/microbench_grader.py::correctness_rate_by_cell` — that
    tells you "is the answer substring-correct", this tells you
    "when both are correct, which one is better".
    """
    summary: dict[str, dict[str, Any]] = {}
    for v in verdicts:
        cell = summary.setdefault(
            v.task_id,
            {"n": 0, "raw_wins": 0, "bg_wins": 0, "ties": 0, "bg_win_rate": 0.0},
        )
        cell["n"] += 1
        if v.winner == "raw":
            cell["raw_wins"] += 1
        elif v.winner == "blastguard":
            cell["bg_wins"] += 1
        else:
            cell["ties"] += 1
    for cell in summary.values():
        if cell["n"] > 0:
            cell["bg_win_rate"] = cell["bg_wins"] / cell["n"]
    return summary
