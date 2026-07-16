#!/usr/bin/env python3
"""Compare observable Ruxel JSON events with an Ansible callback capture."""

from __future__ import annotations

import json
import sys
from pathlib import Path

CORE_FIELDS = {
    "changed", "failed", "skipped", "rc", "stdout", "stderr",
    "stdout_lines", "attempts", "content", "encoding", "diff", "item",
    "results", "stat",
}
STAT_FIELDS = {"exists", "isdir", "islnk", "isblk", "path", "lnk_source"}


def module_name(value):
    if value is None:
        return None
    return str(value).split(".")[-1]


def task_name(value):
    return str(value).split(" : ")[-1]


def normalize_value(value):
    if isinstance(value, list):
        return [normalize_value(item) for item in value]
    if not isinstance(value, dict):
        return value
    normalized = {}
    for key, child in value.items():
        if key not in CORE_FIELDS:
            continue
        if key == "stat" and isinstance(child, dict):
            normalized[key] = {
                field: normalize_value(field_value)
                for field, field_value in child.items()
                if field in STAT_FIELDS
            }
        else:
            normalized[key] = normalize_value(child)
    return normalized


def normalize_ansible(record):
    result = record.get("result") if isinstance(record.get("result"), dict) else {}
    status = record.get("status")
    if status == "ok" and result.get("changed"):
        status = "changed"
    return {
        "task": task_name(record.get("task_name")),
        "module": module_name(record.get("action")),
        "status": status,
        "ignored": bool(record.get("ignore_errors")),
        "result": normalize_value(result),
    }


def normalize_ruxel(record):
    return {
        "task": task_name(record.get("task")),
        "module": module_name(record.get("module")),
        "status": record.get("status"),
        "ignored": bool(record.get("ignored")),
        "result": normalize_value(record.get("result") or {}),
    }


def load_jsonl(path):
    records = []
    for line_number, line in enumerate(Path(path).read_text().splitlines(), 1):
        if not line.strip():
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as error:
            raise SystemExit(f"{path}:{line_number}: invalid JSON: {error}") from error
    return records


def compare(ruxel_path, ansible_path):
    ruxel = [
        normalize_ruxel(record)
        for record in load_jsonl(ruxel_path)
        if record.get("event") == "task"
    ]
    ansible = [
        normalize_ansible(record)
        for record in load_jsonl(ansible_path)
        if record.get("status") in {"ok", "failed", "skipped", "unreachable"}
        and module_name(record.get("action")) not in {"gather_facts", "setup"}
    ]
    if ruxel == ansible:
        print(f"result parity: {len(ruxel)} task outcomes match")
        return 0

    limit = max(len(ruxel), len(ansible))
    for index in range(limit):
        left = ruxel[index] if index < len(ruxel) else "<missing>"
        right = ansible[index] if index < len(ansible) else "<missing>"
        if left != right:
            print(f"task outcome mismatch at index {index}", file=sys.stderr)
            print("ruxel:   " + json.dumps(left, sort_keys=True), file=sys.stderr)
            print("ansible: " + json.dumps(right, sort_keys=True), file=sys.stderr)
    return 1


def main():
    if len(sys.argv) != 3:
        print("usage: compare_results.py <ruxel.jsonl> <ansible.jsonl>", file=sys.stderr)
        return 2
    return compare(sys.argv[1], sys.argv[2])


if __name__ == "__main__":
    raise SystemExit(main())
