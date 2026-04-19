"""Transform SWE-bench Pro HF rows into SWE-agent `SimpleBatchInstance` JSONL.

SWE-agent's `InstancesFromHuggingFace` expects `image_name` in the dataset,
but `ScaleAI/SWE-bench_Pro` ships with `dockerhub_tag` (pointing at
`jefzda/sweap-images:<tag>`). We bridge by pre-processing the rows to a
local JSONL that the `file` loader can consume.

Writes: `<out_path>` — one JSON object per line, fields:
  image_name: str            # jefzda/sweap-images:<dockerhub_tag>
  problem_statement: str     # full PR description
  instance_id: str
  repo_name: str             # empty — repo pre-checked out inside the image
  base_commit: str
  extra_fields: dict         # carries fail_to_pass / pass_to_pass / repo_language
                             # for downstream evaluation and filtering
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path

from datasets import load_dataset

DEFAULT_IMAGE_REGISTRY = "jefzda/sweap-images"


def prepare(
    dataset_name: str = "ScaleAI/SWE-bench_Pro",
    split: str = "test",
    language_filter: str | None = "python",
    limit: int | None = None,
    out_path: Path | None = None,
    image_registry: str = DEFAULT_IMAGE_REGISTRY,
) -> Path:
    if out_path is None:
        out_path = Path(__file__).parent / "results" / "instances.jsonl"
    out_path.parent.mkdir(parents=True, exist_ok=True)

    ds = load_dataset(dataset_name, split=split)
    count = 0
    with out_path.open("w", encoding="utf-8") as f:
        for row in ds:
            if language_filter and str(row.get("repo_language", "")).lower() != language_filter:
                continue
            tag = str(row["dockerhub_tag"]).strip()
            if not tag:
                continue
            image_name = f"{image_registry}:{tag}"
            instance = {
                "image_name": image_name,
                "problem_statement": str(row["problem_statement"]),
                "instance_id": str(row["instance_id"]),
                "repo_name": "",
                "base_commit": str(row["base_commit"]),
                "extra_fields": {
                    "fail_to_pass": row.get("fail_to_pass"),
                    "pass_to_pass": row.get("pass_to_pass"),
                    "repo_language": row.get("repo_language"),
                    "dockerhub_tag": tag,
                    "before_repo_set_cmd": row.get("before_repo_set_cmd"),
                    "selected_test_files_to_run": row.get("selected_test_files_to_run"),
                },
            }
            f.write(json.dumps(instance) + "\n")
            count += 1
            if limit is not None and count >= limit:
                break
    return out_path


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--dataset-name", default="ScaleAI/SWE-bench_Pro")
    p.add_argument("--split", default="test")
    p.add_argument("--language", default="python", help="Set to empty string for all")
    p.add_argument("--limit", type=int, default=None)
    p.add_argument("--out", type=Path, default=None)
    p.add_argument("--image-registry", default=DEFAULT_IMAGE_REGISTRY)
    args = p.parse_args()

    os.environ.setdefault("HF_HOME", "/tmp/hf")
    path = prepare(
        dataset_name=args.dataset_name,
        split=args.split,
        language_filter=args.language or None,
        limit=args.limit,
        out_path=args.out,
        image_registry=args.image_registry,
    )
    count = sum(1 for _ in path.open())
    print(f"wrote {count} instances to {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
