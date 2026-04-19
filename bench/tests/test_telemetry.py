from pathlib import Path

from bench.telemetry import TelemetryRecord, write_jsonl


def test_telemetry_record_roundtrip(tmp_path: Path):
    rec = TelemetryRecord(
        task_id="django__123",
        arm="blastguard",
        input_tokens=100,
        cached_input_tokens=80,
        output_tokens=50,
        turns=10,
        wall_seconds=42.5,
        cost_usd=0.001,
        patch_bytes=512,
        error=None,
    )
    out = tmp_path / "telemetry.jsonl"
    write_jsonl([rec], out)
    assert out.read_text().strip().startswith('{"task_id": "django__123"')
