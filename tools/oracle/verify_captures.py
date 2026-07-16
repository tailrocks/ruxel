#!/usr/bin/env python3
"""Reject stale/non-normalized or machine-specific committed oracle captures."""

import json
import hashlib
import re
import sys
from pathlib import Path

SCHEMA = {"task_name", "action", "status", "result", "ignore_errors", "raw_args"}
FORBIDDEN_KEYS = {
    "delta", "end", "start", "stop", "invocation", "exception", "host",
    "play", "playbook", "resolved_args", "discovered_interpreter_python",
}
FORBIDDEN_VALUES = re.compile(
    r"/var/folders/|/Users/|(?:[0-9a-f]{2}:){5}[0-9a-f]{2}|"
    r"\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b",
    re.IGNORECASE,
)


def required_parity_stems(matrix_path):
    matrix = json.loads(Path(matrix_path).read_text())
    return {Path(entry["playbook"]).stem for entry in matrix}


def walk(value, path=""):
    if isinstance(value, dict):
        for key, child in value.items():
            if key in FORBIDDEN_KEYS or (key.startswith("_ansible") and key != "_ansible_no_log"):
                raise ValueError(f"forbidden key {path}/{key}")
            walk(child, f"{path}/{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            walk(child, f"{path}/{index}")
    elif isinstance(value, str) and FORBIDDEN_VALUES.search(value):
        raise ValueError(f"machine-specific value at {path}")


def main():
    failures = []
    for capture in sorted(Path("tools/oracle/captures").glob("*.jsonl")):
        if capture.name == "render-parity.jsonl":
            continue
        for number, line in enumerate(capture.read_text().splitlines(), 1):
            try:
                record = json.loads(line)
                extra = set(record) - SCHEMA
                if extra:
                    raise ValueError(f"non-normalized fields {sorted(extra)}")
                walk(record)
            except (ValueError, json.JSONDecodeError) as error:
                failures.append(f"{capture}:{number}: {error}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    manifests = sorted(Path("tools/oracle/parity").glob("*.json"))
    required = required_parity_stems("tools/fixtures/parity-matrix.json")
    present = {manifest.stem for manifest in manifests}
    if present != required:
        print(
            "parity manifest matrix mismatch: "
            f"missing={sorted(required - present)} extra={sorted(present - required)}",
            file=sys.stderr,
        )
        return 1
    for manifest in manifests:
        data = json.loads(manifest.read_text())
        stem = data["playbook"]
        if manifest.stem != stem:
            print(f"{manifest}: playbook stem mismatch", file=sys.stderr)
            return 1
        names = {
            "fresh": f"fresh-{stem}.jsonl",
            "converged": f"converged-{stem}.jsonl",
            "check_diff": f"check-{stem}.jsonl",
        }
        for mode, name in names.items():
            capture = Path("tools/oracle/captures") / name
            proof = data["modes"][mode]
            required_proof = (
                {"result_parity": True, "state_parity": True}
                if mode != "check_diff"
                else {"result_parity": True, "state_contract": True}
            )
            if any(proof.get(key) is not value for key, value in required_proof.items()):
                print(f"{manifest}: incomplete {mode} proof", file=sys.stderr)
                return 1
            digest = hashlib.sha256(capture.read_bytes()).hexdigest()
            if digest != data["modes"][mode]["capture_sha256"]:
                print(f"{manifest}: stale {mode} capture hash", file=sys.stderr)
                return 1
    print("oracle captures: normalized, machine-independent, parity hashes current")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
