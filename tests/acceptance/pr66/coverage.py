#!/usr/bin/env python3
"""Check that every concrete Pkl operation has an agent dispatch arm.

This is a structural guard, not proof that a provider works. Controller-only
operations must be added here only when there is an explicit executable route.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SCHEMA = ROOT / "schema/Ruxel.pkl"
AGENT = ROOT / "crates/ruxel/ruxel-agent/src/modules/mod.rs"


def operation_kinds(source):
    classes = re.findall(r"\bclass\s+(\w+)\s+extends\s+Operation\s*\{([^{}]*)\}", source, re.S)
    if not classes:
        raise ValueError("no concrete Operation classes found")
    kinds = []
    for name, body in classes:
        match = re.search(r'\bkind\s*=\s*"([^"]+)"', body)
        if not match:
            raise ValueError(f"{name} has no literal kind")
        kinds.append(match.group(1))
    if len(set(kinds)) != len(kinds):
        raise ValueError("duplicate operation kind")
    return set(kinds)


def dispatch_kinds(source):
    # Limit extraction to execute's actual match, not is_implemented, comments,
    # tests or unrelated string literals. Fail closed if the format changes.
    match = re.search(r"let result = match module\s*\{(.*?)\n\s*_\s*=>", source, re.S)
    if not match:
        raise ValueError("agent dispatch match not found")
    arms = re.findall(r'^\s*((?:"[^"]+"\s*(?:\|\s*)?)+)\s*=>', match.group(1), re.M)
    kinds = {kind for arm in arms for kind in re.findall(r'"([^"]+)"', arm)}
    if not kinds:
        raise ValueError("agent dispatch contains no recognizable arms")
    return kinds


def main():
    try:
        declared = operation_kinds(SCHEMA.read_text())
        implemented = dispatch_kinds(AGENT.read_text())
    except (OSError, ValueError) as error:
        print(f"coverage error: {error}", file=sys.stderr)
        return 1
    missing = sorted(declared - implemented)
    for kind in missing:
        print(f"no executable agent dispatch for schema operation: {kind}", file=sys.stderr)
    if missing:
        return 1
    print(f"{len(declared)} schema operation kinds have agent dispatch arms; behavioral tests still required")
    return 0


if __name__ == "__main__":
    sys.exit(main())
