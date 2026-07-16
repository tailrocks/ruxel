#!/usr/bin/env python3
"""Remove machine/run-specific fields from an Ansible JSONL capture."""

import json
import sys
from pathlib import Path

from compare_results import module_name, normalize_value

DROP = {
    "delta", "start", "end", "stop", "warnings", "invocation", "exception",
    "before_header", "after_header",
    "discovered_interpreter_python",
}


def scrub(value):
    if isinstance(value, list):
        return [scrub(item) for item in value]
    if not isinstance(value, dict):
        return value
    normalized = {
        key: scrub(child)
        for key, child in value.items()
        if key not in DROP and (not key.startswith("_ansible") or key == "_ansible_no_log")
    }
    if "censored" in normalized:
        normalized["_ansible_no_log"] = True
    return normalized


def normalize(record):
    action = module_name(record["action"])
    normalized = {
        "task_name": record["task_name"],
        "action": record["action"],
        "status": record["status"],
        # Captures are durable semantic evidence, not raw Ansible diagnostics.
        # Keep exactly the result surface compared with Ruxel; module temp
        # paths, timestamps, PIDs, inodes, and systemd runtime counters are
        # deliberately excluded.
        "result": normalize_value(scrub(record.get("result") or {}), action),
    }
    if record.get("ignore_errors"):
        normalized["ignore_errors"] = True
    if record.get("raw_args") is not None:
        normalized["raw_args"] = scrub(record["raw_args"])
    return normalized


def main():
    path = Path(sys.argv[1])
    records = []
    for line in path.read_text().splitlines():
        if not line:
            continue
        record = json.loads(line)
        action = str(record.get("action", "")).split(".")[-1]
        if action in {"gather_facts", "setup"}:
            continue
        records.append(normalize(record))
    path.write_text("".join(json.dumps(record, sort_keys=True) + "\n" for record in records))


if __name__ == "__main__":
    main()
