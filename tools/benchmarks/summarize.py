#!/usr/bin/env python3
"""Recompute benchmark summaries from committed raw samples."""

from __future__ import annotations

import argparse
import json
import math
import statistics
from pathlib import Path
from typing import Any

EXECUTORS = ("ansible", "ruxel")
NANOSECOND_STAT_DECIMALS = 6
RATIO_DECIMALS = 12


class EvidenceError(ValueError):
    """Benchmark evidence is malformed."""


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvidenceError(f"cannot read JSON {path}: {error}") from error


def load_samples(path: Path) -> list[dict[str, Any]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as error:
        raise EvidenceError(f"cannot read samples {path}: {error}") from error
    samples = []
    for number, line in enumerate(lines, 1):
        if not line.strip():
            continue
        try:
            value = json.loads(line)
        except json.JSONDecodeError as error:
            raise EvidenceError(f"invalid JSON at {path}:{number}: {error}") from error
        if not isinstance(value, dict):
            raise EvidenceError(f"sample at {path}:{number} is not an object")
        samples.append(value)
    return samples


def stats(values: list[int]) -> dict[str, int | float]:
    if not values:
        raise EvidenceError("cannot summarize zero samples")
    ordered = sorted(values)
    p95_index = max(0, math.ceil(len(ordered) * 0.95) - 1)
    return {
        "n": len(ordered),
        "min_ns": ordered[0],
        "median_ns": statistics.median(ordered),
        "mean_ns": round(statistics.fmean(ordered), NANOSECOND_STAT_DECIMALS),
        "p95_ns": ordered[p95_index],
        "stdev_ns": round(statistics.pstdev(ordered), NANOSECOND_STAT_DECIMALS),
    }


def summarize_samples(samples: list[dict[str, Any]]) -> dict[str, Any]:
    grouped: dict[str, list[int]] = {executor: [] for executor in EXECUTORS}
    for index, sample in enumerate(samples, 1):
        executor = sample.get("executor")
        if executor not in grouped:
            raise EvidenceError(f"sample {index} has invalid executor {executor!r}")
        accepted = sample.get("accepted")
        elapsed = sample.get("elapsed_ns")
        if not isinstance(accepted, bool):
            raise EvidenceError(f"sample {index} accepted is not boolean")
        if not isinstance(elapsed, int) or isinstance(elapsed, bool) or elapsed <= 0:
            raise EvidenceError(f"sample {index} elapsed_ns must be a positive integer")
        if accepted:
            grouped[executor].append(elapsed)

    executors = {executor: stats(grouped[executor]) for executor in EXECUTORS}
    speedup = executors["ansible"]["median_ns"] / executors["ruxel"]["median_ns"]
    return {
        "schema": 1,
        "algorithm": {
            "p95": "nearest-rank",
            "stdev": "population",
            "speedup": "ansible_median_ns/ruxel_median_ns",
        },
        "executors": executors,
        "speedup": round(speedup, RATIO_DECIMALS),
    }


def summarize_case(case_dir: Path, *, write: bool = True) -> dict[str, Any]:
    summary = summarize_samples(load_samples(case_dir / "samples.jsonl"))
    manifest = load_json(case_dir / "manifest.json")
    if not isinstance(manifest, dict) or not isinstance(manifest.get("case"), str):
        raise EvidenceError(f"invalid manifest in {case_dir}")
    summary["case"] = manifest["case"]
    if write:
        (case_dir / "summary.json").write_text(
            json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    return summary


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("case_dirs", nargs="+", type=Path)
    parser.add_argument("--check", action="store_true", help="verify existing summaries")
    args = parser.parse_args()
    for case_dir in args.case_dirs:
        expected = summarize_case(case_dir, write=not args.check)
        if args.check and load_json(case_dir / "summary.json") != expected:
            raise EvidenceError(f"stale summary: {case_dir / 'summary.json'}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except EvidenceError as error:
        raise SystemExit(f"benchmark summary: {error}") from error
