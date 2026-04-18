"""Budget cap + per-rollout cost tracking.

Input/output prices are per million tokens. Cache-read price is optional
(defaults to base input price if not supplied). Raises `BudgetExceeded`
the moment a `record()` call would push `spent_usd` past `cap_usd`.
"""

from __future__ import annotations


class BudgetExceeded(RuntimeError):
    """Raised when a record() call would exceed the configured cap."""


class Budget:
    def __init__(self, cap_usd: float) -> None:
        if cap_usd <= 0:
            raise ValueError("cap_usd must be positive")
        self.cap_usd = cap_usd
        self.spent_usd = 0.0

    def record(
        self,
        *,
        input_tokens: int,
        output_tokens: int,
        in_price_per_m: float,
        out_price_per_m: float,
        cached_input_tokens: int = 0,
        cache_read_per_m: float | None = None,
    ) -> float:
        """Charge this call to the budget. Returns the cost of this call."""
        uncached_input = max(0, input_tokens - cached_input_tokens)
        cache_rate = cache_read_per_m if cache_read_per_m is not None else in_price_per_m
        cost = (
            uncached_input * in_price_per_m / 1_000_000.0
            + cached_input_tokens * cache_rate / 1_000_000.0
            + output_tokens * out_price_per_m / 1_000_000.0
        )
        if self.spent_usd + cost > self.cap_usd:
            raise BudgetExceeded(
                f"next call costs ${cost:.4f}; spent ${self.spent_usd:.4f}; cap ${self.cap_usd:.2f}"
            )
        self.spent_usd += cost
        return cost
