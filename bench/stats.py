"""Paired McNemar's test for A/B benchmark comparison.

Given per-task pass/fail outcomes for two arms (raw, blastguard) on the
same tasks, McNemar's chi-squared on discordant pairs tells us whether
one arm flips more tasks net-positive than the other.

Concordant pairs (both pass, both fail) are ignored — they carry no
information about the treatment effect. The test statistic is built
from `b` (only raw passes) and `c` (only blastguard passes):

    chi2 = (|b - c| - 1)^2 / (b + c)     (continuity-corrected)

Small-sample case (b + c < 25): use scipy's exact binomial.
"""

from __future__ import annotations

from dataclasses import dataclass

from scipy.stats import binomtest, chi2


@dataclass(frozen=True, slots=True)
class PairedResult:
    both_pass: int
    both_fail: int
    raw_wins: int           # only raw passed
    blastguard_wins: int    # only blastguard passed
    n: int
    raw_score_pct: float
    blastguard_score_pct: float
    delta_pct: float
    p_value: float
    test_used: str


def mcnemar_paired(pairs: list[tuple[str, int]]) -> PairedResult:
    """Compute McNemar's test from a list of (bucket_name, count) tuples.

    Bucket names: "both_pass", "both_fail", "raw_only_pass",
    "blastguard_only_pass".
    """
    counts = {name: 0 for name in ("both_pass", "both_fail", "raw_only_pass", "blastguard_only_pass")}
    for name, n in pairs:
        if name not in counts:
            raise ValueError(f"unknown bucket: {name}")
        counts[name] = n

    b = counts["raw_only_pass"]
    c = counts["blastguard_only_pass"]
    n_total = sum(counts.values())

    if b + c == 0:
        p_value = 1.0
        test_used = "degenerate (no discordant pairs)"
    elif b + c < 25:
        # exact binomial with p=0.5
        res = binomtest(min(b, c), n=b + c, p=0.5, alternative="two-sided")
        p_value = res.pvalue
        test_used = "exact binomial"
    else:
        stat = (abs(b - c) - 1) ** 2 / (b + c)
        p_value = 1.0 - chi2.cdf(stat, df=1)
        test_used = "chi-squared (continuity-corrected)"

    raw_pass = counts["both_pass"] + b
    blastguard_pass = counts["both_pass"] + c
    raw_pct = 100.0 * raw_pass / n_total if n_total else 0.0
    blastguard_pct = 100.0 * blastguard_pass / n_total if n_total else 0.0

    return PairedResult(
        both_pass=counts["both_pass"],
        both_fail=counts["both_fail"],
        raw_wins=b,
        blastguard_wins=c,
        n=n_total,
        raw_score_pct=raw_pct,
        blastguard_score_pct=blastguard_pct,
        delta_pct=blastguard_pct - raw_pct,
        p_value=p_value,
        test_used=test_used,
    )
