"""Shared per-rollout token count dataclass."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class TokenCount:
    """Aggregated token usage for a single task rollout."""

    input: int
    cached_input: int
    output: int
    turns: int
