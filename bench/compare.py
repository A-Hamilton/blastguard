"""Compare two JSONL result sets and print delta metrics."""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from pathlib import Path

import click


@dataclass(frozen=True, slots=True)
class Summary:
    total: int
    resolved: int
    tampered: int
    total_tokens_in: int
    total_tokens_out: int
    total_turns: int
    per_repo: dict[str, tuple[int, int]]  # repo -> (resolved, total)

    @property
    def resolution_rate(self) -> float:
        return (self.resolved / self.total) if self.total else 0.0


def load_results(path: Path) -> Summary:
    total = 0
    resolved = 0
    tampered = 0
    tokens_in = 0
    tokens_out = 0
    turns = 0
    per_repo: dict[str, list[int]] = {}
    with path.open(encoding="utf-8") as f:
        for line in f:
            if not line.strip():
                continue
            row = json.loads(line)
            total += 1
            if row.get("resolved"):
                resolved += 1
            if row.get("tampered"):
                tampered += 1
            tokens_in += int(row.get("tokens_in", 0))
            tokens_out += int(row.get("tokens_out", 0))
            turns += int(row.get("turns", 0))
            repo = row.get("repo", "unknown")
            per_repo.setdefault(repo, [0, 0])
            per_repo[repo][1] += 1
            if row.get("resolved"):
                per_repo[repo][0] += 1
    return Summary(
        total=total,
        resolved=resolved,
        tampered=tampered,
        total_tokens_in=tokens_in,
        total_tokens_out=tokens_out,
        total_turns=turns,
        per_repo={k: (v[0], v[1]) for k, v in per_repo.items()},
    )


def render_comparison(baseline: Summary, blastguard: Summary) -> str:
    lines: list[str] = []
    lines.append("=== Comparison ===")
    lines.append(
        f"Resolution rate: {baseline.resolution_rate:.1%} → "
        f"{blastguard.resolution_rate:.1%} "
        f"(Δ {(blastguard.resolution_rate - baseline.resolution_rate) * 100:+.1f} pp)"
    )
    lines.append(
        f"Resolved: {baseline.resolved}/{baseline.total} → "
        f"{blastguard.resolved}/{blastguard.total}"
    )
    lines.append(f"Tampered: {baseline.tampered} → {blastguard.tampered}")
    lines.append(
        f"Tokens in (total): {baseline.total_tokens_in:,} → {blastguard.total_tokens_in:,} "
        f"(Δ {blastguard.total_tokens_in - baseline.total_tokens_in:+,})"
    )
    lines.append(
        f"Tokens out (total): {baseline.total_tokens_out:,} → {blastguard.total_tokens_out:,} "
        f"(Δ {blastguard.total_tokens_out - baseline.total_tokens_out:+,})"
    )
    lines.append(
        f"Turns (total): {baseline.total_turns} → {blastguard.total_turns} "
        f"(Δ {blastguard.total_turns - baseline.total_turns:+})"
    )
    lines.append("")
    lines.append("Per-repo:")
    all_repos = sorted(set(baseline.per_repo) | set(blastguard.per_repo))
    for repo in all_repos:
        b = baseline.per_repo.get(repo, (0, 0))
        bg = blastguard.per_repo.get(repo, (0, 0))
        lines.append(f"  {repo}: {b[0]}/{b[1]} → {bg[0]}/{bg[1]}")
    return "\n".join(lines)


@click.command()
@click.argument("baseline_path", type=click.Path(exists=True, path_type=Path))
@click.argument("blastguard_path", type=click.Path(exists=True, path_type=Path))
def main(baseline_path: Path, blastguard_path: Path) -> None:
    baseline = load_results(baseline_path)
    blastguard = load_results(blastguard_path)
    sys.stdout.write(render_comparison(baseline, blastguard) + "\n")


if __name__ == "__main__":
    main()
