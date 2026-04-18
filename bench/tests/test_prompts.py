from bench.prompts import build_system_prompt


def test_raw_prompt_has_no_blastguard_references():
    p = build_system_prompt(arm="raw")
    assert "BlastGuard" not in p
    assert "blastguard__" not in p


def test_blastguard_prompt_includes_all_three_tools():
    p = build_system_prompt(arm="blastguard")
    assert "blastguard__search" in p
    assert "blastguard__apply_change" in p
    assert "blastguard__run_tests" in p
    assert "cascade" in p.lower()


def test_unknown_arm_raises():
    import pytest
    with pytest.raises(ValueError):
        build_system_prompt(arm="nonsense")
