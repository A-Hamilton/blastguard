from bench.prompts import build_system_prompt


def test_raw_prompt_has_no_blastguard_references():
    p = build_system_prompt(arm="raw")
    assert "BlastGuard" not in p
    assert "blastguard__" not in p


def test_blastguard_prompt_includes_all_three_tools():
    p = build_system_prompt(arm="blastguard")
    # SWE-agent bundle exposes tools by their bin/ filename (single underscore).
    assert "blastguard_search" in p
    assert "blastguard_apply_change" in p
    assert "blastguard_run_tests" in p
    assert "cascade" in p.lower()


def test_unknown_arm_raises():
    import pytest
    with pytest.raises(ValueError):
        build_system_prompt(arm="nonsense")
