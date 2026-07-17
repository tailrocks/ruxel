#!/usr/bin/env python3
"""Sanitize and register one benchmark sample."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

IPV4 = re.compile(r"(?<![0-9a-f])(?:25[0-5]|2[0-4]\d|1?\d?\d)(?:\.(?:25[0-5]|2[0-4]\d|1?\d?\d)){3}(?![0-9a-f])")
CONTROLLER_PATH = re.compile(r"(?:/Users/[^/\s]+|/home/[^/\s]+|[A-Za-z]:\\Users\\[^\\\s]+)")


def sanitize(text: str) -> str:
    return CONTROLLER_PATH.sub("<controller-home>", IPV4.sub("<fixture-ip>", text))


def sanitize_file(path: Path) -> str:
    path.write_text(sanitize(path.read_text(encoding="utf-8", errors="replace")), encoding="utf-8")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def append_sample(
    case_dir: Path,
    executor: str,
    repetition: int,
    execution_order: int,
    elapsed_ns: int,
    stdout: Path,
    stderr: Path,
) -> None:
    record = {
        "executor": executor,
        "repetition": repetition,
        "execution_order": execution_order,
        "accepted": True,
        "elapsed_ns": elapsed_ns,
    }
    for stream, path in (("stdout", stdout), ("stderr", stderr)):
        digest = sanitize_file(path)
        record[stream] = {
            "path": path.relative_to(case_dir).as_posix(),
            "sha256": digest,
        }
    with (case_dir / "samples.jsonl").open("a", encoding="utf-8") as output:
        output.write(json.dumps(record, sort_keys=True) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("case_dir", type=Path)
    parser.add_argument("executor", choices=("ansible", "ruxel"))
    parser.add_argument("repetition", type=int)
    parser.add_argument("execution_order", type=int, choices=(1, 2))
    parser.add_argument("elapsed_ns", type=int)
    parser.add_argument("stdout", type=Path)
    parser.add_argument("stderr", type=Path)
    args = parser.parse_args()
    append_sample(
        args.case_dir,
        args.executor,
        args.repetition,
        args.execution_order,
        args.elapsed_ns,
        args.stdout,
        args.stderr,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
