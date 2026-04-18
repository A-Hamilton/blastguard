from bench.stats import mcnemar_paired, PairedResult


def test_mcnemar_detects_positive_lift():
    """BlastGuard flips 20 tasks to pass, raw flips 5. Highly significant."""
    pairs = [
        ("raw_only_pass", 5),
        ("blastguard_only_pass", 20),
        ("both_pass", 60),
        ("both_fail", 15),
    ]
    r = mcnemar_paired(pairs)
    assert r.blastguard_wins == 20
    assert r.raw_wins == 5
    assert r.p_value < 0.01
    assert r.blastguard_score_pct > r.raw_score_pct


def test_mcnemar_detects_no_lift():
    """Symmetric discordant pairs — no signal."""
    pairs = [
        ("raw_only_pass", 10),
        ("blastguard_only_pass", 10),
        ("both_pass", 50),
        ("both_fail", 30),
    ]
    r = mcnemar_paired(pairs)
    assert r.p_value > 0.1


def test_mcnemar_zero_discordant():
    """Identical arms — p-value = 1.0 (degenerate case)."""
    pairs = [("both_pass", 100), ("both_fail", 50), ("raw_only_pass", 0), ("blastguard_only_pass", 0)]
    r = mcnemar_paired(pairs)
    assert r.p_value == 1.0
