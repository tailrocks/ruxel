#!/usr/bin/env python3
"""Remove machine/run-specific fields from an Ansible JSONL capture."""

import json
import sys
from pathlib import Path

DROP = {
    "delta", "start", "end", "warnings", "invocation", "exception",
    "discovered_interpreter_python",
}


def scrub(value):
    if isinstance(value, list):
        return [scrub(item) for item in value]
    if not isinstance(value, dict):
        return value
    return {
        key: scrub(child)
        for key, child in value.items()
        if key not in DROP and not key.startswith("_ansible")
    }


def normalize(record):
    normalized = {
        "task_name": record["task_name"],
        "action": record["action"],
        "status": record["status"],
        "result": scrub(record.get("result") or {}),
    }
    if record.get("ignore_errors"):
        normalized["ignore_errors"] = True
    if record.get("raw_args") is not None:
        normalized["raw_args"] = scrub(record["raw_args"])
    return normalized


def main():
    path = Path(sys.argv[1])
    records = [normalize(json.loads(line)) for line in path.read_text().splitlines() if line]
    path.write_text("".join(json.dumps(record, sort_keys=True) + "\n" for record in records))


if __name__ == "__main__":
    main()
