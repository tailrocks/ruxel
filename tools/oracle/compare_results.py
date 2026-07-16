#!/usr/bin/env python3
"""Compare observable Ruxel JSON events with an Ansible callback capture."""

from __future__ import annotations

import json
import sys
from pathlib import Path

STAT_FIELDS = {"exists", "isdir", "islnk", "isblk", "path", "lnk_source"}


def normalize_diff(value):
    """Represent Ansible structured and Ruxel unified diffs identically."""
    changes = []
    if isinstance(value, str):
        before, after = [], []
        for line in value.splitlines():
            if line.startswith("---") or line.startswith("+++"):
                continue
            if line.startswith("-"):
                before.append(line[1:])
            elif line.startswith("+"):
                after.append(line[1:])
        if before or after:
            changes.append({"before": before, "after": after})
    elif isinstance(value, list):
        for item in value:
            if not isinstance(item, dict) or "before" not in item or "after" not in item:
                continue
            before = str(item["before"]).splitlines()
            after = str(item["after"]).splitlines()
            if before or after:
                changes.append({"before": before, "after": after})
    return changes


def module_name(value):
    if value is None:
        return None
    return str(value).split(".")[-1]


def task_name(value):
    return str(value).split(" : ")[-1]


def normalize_value(value, module=None, include_diff=True):
    if isinstance(value, list):
        return [normalize_value(item, module, include_diff) for item in value]
    if not isinstance(value, dict):
        return value
    allowed = {"changed", "item", "results", "diff"}
    if module in {"command", "shell"}:
        allowed.update({"rc", "stdout", "stderr", "stdout_lines", "attempts"})
    elif module == "stat":
        allowed.add("stat")
    elif module == "slurp":
        allowed.update({"content", "encoding"})
    elif module == "set_fact":
        allowed.add("ansible_facts")
    normalized = {}
    if "censored" in value:
        normalized["redacted"] = True
    for key, child in value.items():
        if key not in allowed:
            continue
        if key == "diff":
            if not include_diff:
                continue
            normalized_child = normalize_diff(child)
            if normalized_child:
                normalized[key] = normalized_child
            continue
        normalized_child = normalize_value(child, module, include_diff)
        if key == "stat" and isinstance(child, dict):
            normalized[key] = {
                field: normalize_value(field_value, module)
                for field, field_value in child.items()
                if field in STAT_FIELDS
            }
        elif key == "ansible_facts":
            normalized[key] = child
        else:
            normalized[key] = normalized_child
    return normalized


def normalize_ansible(record, include_diff=True):
    result = record.get("result") if isinstance(record.get("result"), dict) else {}
    status = record.get("status")
    if status == "ok" and result.get("changed"):
        status = "changed"
    module = module_name(record.get("action"))
    return {
        "task": task_name(record.get("task_name")),
        "module": module,
        "status": status,
        "ignored": bool(record.get("ignore_errors")),
        "result": normalize_value(result, module, include_diff),
    }


def normalize_ruxel(record, include_diff=True):
    module = module_name(record.get("module"))
    return {
        "task": task_name(record.get("task")),
        "module": module,
        "status": record.get("status"),
        "ignored": bool(record.get("ignored")),
        "result": normalize_value(record.get("result") or {}, module, include_diff),
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


def compare(ruxel_path, ansible_path, include_diff=True):
    ruxel = [
        normalize_ruxel(record, include_diff)
        for record in load_jsonl(ruxel_path)
        if record.get("event") == "task"
    ]
    ansible_records = load_jsonl(ansible_path)
    validate_item_aggregates(ansible_records)
    ansible = [
        normalize_ansible(record, include_diff)
        for record in ansible_records
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


def validate_item_aggregates(records):
    pending = {}
    for record in records:
        status = record.get("status", "")
        task = task_name(record.get("task_name"))
        if status.startswith("item_"):
            pending.setdefault(task, []).append(record)
            continue
        items = pending.pop(task, [])
        if not items:
            continue
        result = record.get("result") or {}
        aggregate = result.get("results") if isinstance(result, dict) else None
        if not isinstance(aggregate, list) or len(aggregate) != len(items):
            raise SystemExit(f"Ansible item/aggregate mismatch for {task}")
        module = module_name(record.get("action"))
        for index, (item_record, aggregate_result) in enumerate(zip(items, aggregate)):
            if normalize_value(item_record.get("result") or {}, module) != normalize_value(aggregate_result, module):
                raise SystemExit(f"Ansible item result mismatch for {task}[{index}]")


def main():
    args = [arg for arg in sys.argv[1:] if arg != "--ignore-diffs"]
    if len(args) != 2:
        print("usage: compare_results.py [--ignore-diffs] <ruxel.jsonl> <ansible.jsonl>", file=sys.stderr)
        return 2
    return compare(args[0], args[1], "--ignore-diffs" not in sys.argv)


if __name__ == "__main__":
    raise SystemExit(main())
