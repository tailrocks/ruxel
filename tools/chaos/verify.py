#!/usr/bin/env python3
"""Verify committed synthetic SSH-chaos acceptance evidence."""

import ipaddress
import hashlib
import json
import re
import sys
from pathlib import Path

from make_payload import payload_bytes
from make_playbook import render_playbook


REQUIRED_CASES = {
    "upload-start",
    "partial-hello-ack",
    "large-plan",
    "large-task-result",
    "long-subprocess",
    "controlmaster-sigint",
}
TOP_LEVEL_KEYS = {
    "schema_version", "target", "fixture_source", "fixture_source_sha256",
    "generated_playbook_sha256", "binaries", "versions", "fixture", "cases",
}
SHA256 = re.compile(r"^[0-9a-f]{64}$")
CASE_KEYS = {
    "case",
    "injection_sentinel",
    "interrupted_status",
    "reconnect",
    "flock_free",
    "converged",
    "converged_changed",
    "converged_failed",
    "state_equal",
    "no_process_leak",
    "no_socket_leak",
    "no_temp_leak",
    "recovery_elapsed_ms",
    "recovery_timeout_ms",
}
TRUE_PROOFS = {
    "injection_sentinel",
    "reconnect",
    "flock_free",
    "converged",
    "state_equal",
    "no_process_leak",
    "no_socket_leak",
    "no_temp_leak",
}
MAX_RECOVERY_TIMEOUT_MS = 120_000
CONTROLLER_PATH = re.compile(r"(?:^|[\\/])(?:Users|home|var[\\/]folders)[\\/]", re.I)
SECRET_SHAPE = re.compile(
    r"(?:op://|BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY|"
    r"(?:password|passwd|secret|token|api[_-]?key)\s*[:=])",
    re.I,
)


def _reject_unsafe_string(value: str, location: str) -> None:
    if CONTROLLER_PATH.search(value):
        raise ValueError(f"controller path at {location}")
    if SECRET_SHAPE.search(value):
        raise ValueError(f"secret-shaped value at {location}")
    candidate = value.strip("[]")
    try:
        ipaddress.ip_address(candidate)
    except ValueError:
        return
    raise ValueError(f"IP address at {location}")


def _walk_strings(value, location="$") -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            _reject_unsafe_string(str(key), f"{location}.<key>")
            _walk_strings(child, f"{location}.{key}")
    elif isinstance(value, list):
        for index, child in enumerate(value):
            _walk_strings(child, f"{location}[{index}]")
    elif isinstance(value, str):
        _reject_unsafe_string(value, location)


def validate(data) -> None:
    if not isinstance(data, dict):
        raise ValueError("manifest must be an object")
    if set(data) != TOP_LEVEL_KEYS:
        raise ValueError(
            f"top-level fields must be exactly {sorted(TOP_LEVEL_KEYS)}"
        )
    if data["schema_version"] != 1:
        raise ValueError("schema_version must be 1")
    if data["target"] != "<fixture>":
        raise ValueError("target must be normalized as <fixture>")
    if data["fixture_source"] != "tools/fixture-project/chaos/chaos.yml":
        raise ValueError("fixture source must be the committed chaos fixture")
    source = Path(data["fixture_source"])
    source_digest = hashlib.sha256(source.read_bytes()).hexdigest()
    if data["fixture_source_sha256"] != source_digest:
        raise ValueError("fixture source hash mismatch")
    generated_digest = hashlib.sha256(
        render_playbook(source.read_text(), payload_bytes()).encode()
    ).hexdigest()
    if data["generated_playbook_sha256"] != generated_digest:
        raise ValueError("generated playbook hash mismatch")
    binaries = data["binaries"]
    if not isinstance(binaries, dict) or set(binaries) != {
        "controller_sha256", "agent_sha256"
    } or any(not SHA256.fullmatch(str(value)) for value in binaries.values()):
        raise ValueError("binary hashes must be exact SHA-256 fields")
    parity = json.loads(Path("tools/oracle/parity/control-flow.json").read_text())
    if binaries != parity.get("binaries"):
        raise ValueError("chaos binaries do not match current parity evidence")
    versions = data["versions"]
    if not isinstance(versions, dict) or set(versions) != {"ruxel", "rustc"} or any(
        not isinstance(value, str) or not value for value in versions.values()
    ):
        raise ValueError("versions must contain non-empty ruxel and rustc strings")
    fixture = data["fixture"]
    if not isinstance(fixture, dict) or set(fixture) != {"kind", "specification"}:
        raise ValueError("fixture metadata fields are invalid")
    if fixture["kind"] != "disposable-provider" or not isinstance(
        fixture["specification"], dict
    ) or not fixture["specification"]:
        raise ValueError("fixture specification must identify a disposable provider target")
    if not isinstance(data["cases"], list):
        raise ValueError("cases must be an array")

    names = []
    for index, case in enumerate(data["cases"]):
        location = f"cases[{index}]"
        if not isinstance(case, dict):
            raise ValueError(f"{location} must be an object")
        if set(case) != CASE_KEYS:
            missing = sorted(CASE_KEYS - set(case))
            extra = sorted(set(case) - CASE_KEYS)
            raise ValueError(f"{location} fields: missing={missing} extra={extra}")
        name = case["case"]
        if name not in REQUIRED_CASES:
            raise ValueError(f"{location}.case is not canonical: {name!r}")
        names.append(name)
        for field in TRUE_PROOFS:
            if case[field] is not True:
                raise ValueError(f"{location}.{field} must be true")
        status = case["interrupted_status"]
        if isinstance(status, bool) or not isinstance(status, int) or status == 0:
            raise ValueError(f"{location}.interrupted_status must be a nonzero integer")
        for field in ("converged_changed", "converged_failed"):
            value = case[field]
            if isinstance(value, bool) or value != 0:
                raise ValueError(f"{location}.{field} must be integer 0")
        elapsed = case["recovery_elapsed_ms"]
        timeout = case["recovery_timeout_ms"]
        if any(isinstance(value, bool) or not isinstance(value, int) for value in (elapsed, timeout)):
            raise ValueError(f"{location} recovery bounds must be integers")
        if not 0 <= elapsed <= timeout:
            raise ValueError(f"{location} recovery exceeded its timeout")
        if not 1 <= timeout <= MAX_RECOVERY_TIMEOUT_MS:
            raise ValueError(
                f"{location}.recovery_timeout_ms must be 1..{MAX_RECOVERY_TIMEOUT_MS}"
            )

    present = set(names)
    if len(names) != len(present):
        raise ValueError("chaos cases must not be duplicated")
    if present != REQUIRED_CASES:
        raise ValueError(
            "chaos case matrix mismatch: "
            f"missing={sorted(REQUIRED_CASES - present)} "
            f"extra={sorted(present - REQUIRED_CASES)}"
        )
    _walk_strings(data)


def main(argv=None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    path = Path(argv[0]) if argv else Path("tools/chaos/artifacts/manifest.json")
    if len(argv) > 1:
        print(f"usage: {Path(sys.argv[0]).name} [manifest.json]", file=sys.stderr)
        return 2
    try:
        validate(json.loads(path.read_text()))
    except (OSError, json.JSONDecodeError, ValueError) as error:
        print(f"{path}: {error}", file=sys.stderr)
        return 1
    print("chaos evidence: complete, bounded, normalized, leak-free")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
