#!/usr/bin/env python3
"""Materialize the deterministic large-Plan fixture payload."""

import sys
from pathlib import Path


SIZE = 2 * 1024 * 1024


def payload_bytes():
    return b"P" * SIZE


def write_payload(path):
    Path(path).write_bytes(payload_bytes())


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} OUTPUT")
    write_payload(sys.argv[1])
