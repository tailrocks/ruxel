#!/usr/bin/env python3
"""Run one benchmark command and record monotonic elapsed nanoseconds."""

from __future__ import annotations

import argparse
import subprocess
import time
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("elapsed", type=Path)
    parser.add_argument("status", type=Path)
    parser.add_argument("stdout", type=Path)
    parser.add_argument("stderr", type=Path)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        parser.error("command required after --")
    start = time.monotonic_ns()
    with args.stdout.open("wb") as stdout, args.stderr.open("wb") as stderr:
        result = subprocess.run(command, stdout=stdout, stderr=stderr, check=False)
    elapsed = time.monotonic_ns() - start
    args.elapsed.write_text(f"{elapsed}\n", encoding="ascii")
    args.status.write_text(f"{result.returncode}\n", encoding="ascii")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
