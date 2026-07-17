#!/usr/bin/env python3
"""Validate the bounded synthetic performance-scale fixture."""

import re
import sys
from pathlib import Path

TASK = re.compile(r"^    - name: ", re.MULTILINE)
ASSERT_ACTION = re.compile(r"^      assert:$", re.MULTILINE)
COMMAND_ACTION = re.compile(r"^      command:$", re.MULTILINE)
NO_LOG = re.compile(r"^      no_log: true$", re.MULTILINE)
LOOKUP = re.compile(r"lookup\(\s*['\"]community\.general\.onepassword['\"]")
SECRET_NAME = re.compile(r"^    benchmark_secret_(\d{2}):", re.MULTILINE)
SYNTHETIC_ITEM = re.compile(r"'ruxel-benchmark-item-(\d{2})'")
FORBIDDEN = re.compile(
    r"(?:op://|ChainArgos|java-monorepo|hosts\.ini|"
    r"\b(?:titan|delorean|pegasus|sentry|postgresql-nova|clickhouse-selene)\b|"
    r"(?:\d{1,3}\.){3}\d{1,3})",
    re.IGNORECASE,
)


def validate(path: Path) -> list[str]:
    text = path.read_text()
    errors = []
    task_count = len(TASK.findall(text))
    lookup_count = len(LOOKUP.findall(text))
    secret_names = SECRET_NAME.findall(text)
    item_names = SYNTHETIC_ITEM.findall(text)
    expected = [f"{number:02d}" for number in range(1, 53)]

    if task_count != 65:
        errors.append(f"expected 65 tasks, found {task_count}")
    if lookup_count != 52:
        errors.append(f"expected 52 onepassword lookups, found {lookup_count}")
    if len(ASSERT_ACTION.findall(text)) != 52:
        errors.append("expected exactly 52 closed-surface assert tasks")
    if len(COMMAND_ACTION.findall(text)) != 13:
        errors.append("expected exactly 13 closed-surface command tasks")
    if len(NO_LOG.findall(text)) != 52:
        errors.append("all 52 secret-consuming tasks must use no_log")
    if secret_names != expected:
        errors.append("secret variable sequence must be exactly 01..52")
    if item_names != expected:
        errors.append("synthetic secret item sequence must be exactly 01..52")
    if "hosts: benchmark" not in text:
        errors.append("fixture must target only the synthetic benchmark group")
    if "vault='ruxel-benchmark'" not in text:
        errors.append("fixture must use the synthetic benchmark vault")
    match = FORBIDDEN.search(text)
    if match:
        errors.append(f"forbidden real identity or address marker: {match.group(0)}")
    return errors


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print(f"usage: {argv[0]} FIXTURE", file=sys.stderr)
        return 2
    errors = validate(Path(argv[1]))
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("benchmark fixture: 65 tasks, 52 synthetic lookups, identities clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
