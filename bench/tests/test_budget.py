import pytest

from bench.budget import Budget, BudgetExceeded


def test_budget_records_costs():
    b = Budget(cap_usd=1.00)
    b.record(input_tokens=1000, output_tokens=500, in_price_per_m=0.30, out_price_per_m=1.20)
    # 1000/1M * 0.30 + 500/1M * 1.20 = 0.0003 + 0.0006 = 0.0009
    assert abs(b.spent_usd - 0.0009) < 1e-6


def test_budget_aborts_when_cap_exceeded():
    b = Budget(cap_usd=0.001)
    with pytest.raises(BudgetExceeded):
        b.record(input_tokens=100_000, output_tokens=50_000, in_price_per_m=0.30, out_price_per_m=1.20)


def test_budget_cache_reads_are_cheaper():
    b = Budget(cap_usd=10.00)
    b.record(
        input_tokens=1_000_000,
        cached_input_tokens=750_000,
        output_tokens=100_000,
        in_price_per_m=0.30,
        cache_read_per_m=0.075,
        out_price_per_m=1.20,
    )
    # uncached 250k * 0.30 + cached 750k * 0.075 + output 100k * 1.20
    # = 0.075 + 0.05625 + 0.12 = 0.25125
    assert abs(b.spent_usd - 0.25125) < 1e-6
