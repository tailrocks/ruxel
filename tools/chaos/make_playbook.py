#!/usr/bin/env python3
"""Materialize the deterministic large-Plan chaos playbook."""

import json
import sys
from pathlib import Path


NEEDLE = "    - name: Large Plan payload"


def render_playbook(source: str, payload: bytes) -> str:
    if source.count(NEEDLE) != 1:
        raise ValueError("chaos playbook must contain one large-Plan marker")
    name = "Large Plan payload " + payload.decode()
    return source.replace(NEEDLE, "    - name: " + json.dumps(name))


if __name__ == "__main__":
    if len(sys.argv) != 4:
        raise SystemExit(f"usage: {Path(sys.argv[0]).name} SOURCE PAYLOAD OUTPUT")
    source, payload, output = map(Path, sys.argv[1:])
    output.write_text(render_playbook(source.read_text(), payload.read_bytes()))
