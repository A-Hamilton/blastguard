"""Per-rollout telemetry JSONL writer."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class TelemetryRecord:
    task_id: str
    arm: str                    # "raw" or "blastguard"
    input_tokens: int
    cached_input_tokens: int
    output_tokens: int
    turns: int
    wall_seconds: float
    cost_usd: float
    patch_bytes: int
    error: str | None


def write_jsonl(records: list[TelemetryRecord], path: Path) -> None:
    with path.open("w", encoding="utf-8") as f:
        for r in records:
            f.write(json.dumps(asdict(r)) + "\n")


def append_jsonl(record: TelemetryRecord, path: Path) -> None:
    with path.open("a", encoding="utf-8") as f:
        f.write(json.dumps(asdict(record)) + "\n")
