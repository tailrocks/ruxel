#!/usr/bin/env python3
"""Compare Ruxel JSON diffs with diffs Ansible actually printed."""

import json
import re
import sys
from pathlib import Path

TASK = re.compile(r"^(?:TASK|RUNNING HANDLER) \[(.+)] \*+$")


def ansible_diffs(text):
    task = None
    current = None
    result = {}
    for line in text.splitlines():
        match = TASK.match(line)
        if match:
            task = match.group(1)
            current = None
        elif line.startswith("--- "):
            current = {"before": [], "after": []}
            result.setdefault(task, []).append(current)
        elif current is not None and line.startswith("-") and not line.startswith("---"):
            current["before"].append(line[1:])
        elif current is not None and line.startswith("+") and not line.startswith("+++"):
            current["after"].append(line[1:])
    return {task: changes for task, changes in result.items()
            if any(change["before"] or change["after"] for change in changes)}


def ruxel_diffs(path):
    result = {}
    for line in Path(path).read_text().splitlines():
        record = json.loads(line)
        if record.get("event") != "task":
            continue
        diff = (record.get("result") or {}).get("diff")
        if not isinstance(diff, str) or not diff:
            continue
        before, after = [], []
        for diff_line in diff.splitlines():
            if diff_line.startswith("-") and not diff_line.startswith("---"):
                before.append(diff_line[1:])
            elif diff_line.startswith("+") and not diff_line.startswith("+++"):
                after.append(diff_line[1:])
        if before or after:
            result.setdefault(record["task"].split(" : ")[-1], []).append(
                {"before": before, "after": after})
    return result


def main():
    left = ruxel_diffs(sys.argv[1])
    right = ansible_diffs(Path(sys.argv[2]).read_text())
    if left != right:
        print("diff mismatch", file=sys.stderr)
        print("ruxel: " + json.dumps(left, sort_keys=True), file=sys.stderr)
        print("ansible: " + json.dumps(right, sort_keys=True), file=sys.stderr)
        return 1
    print(f"diff parity: {len(left)} task diffs match")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
