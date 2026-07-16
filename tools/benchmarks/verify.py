#!/usr/bin/env python3
"""Verify completeness, integrity, safety, and statistics of benchmark artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from summarize import EXECUTORS, EvidenceError, load_json, load_samples, summarize_samples  # noqa: E402

REQUIRED_CASES = {
    "fresh",
    "converged",
    "one-task-drift",
    "check-diff",
    "secret",
    "storage",
    "postgresql",
    "simulated-rtt",
    "six-host",
}
REQUIRED_CORRECTNESS = {
    "fixture_identity_verified",
    "result_parity",
    "diff_parity",
    "state_parity",
    "resources_reaped",
}
REQUIRED_BINARIES = {"controller_sha256", "agent_sha256"}
REQUIRED_VERSIONS = {"ansible", "ruxel", "agent", "rustc", "os", "kernel"}
SHA256 = re.compile(r"^[0-9a-f]{64}$")
IPV4 = re.compile(r"(?<![0-9a-f])(?:25[0-5]|2[0-4]\d|1?\d?\d)(?:\.(?:25[0-5]|2[0-4]\d|1?\d?\d)){3}(?![0-9a-f])")
IPV6 = re.compile(r"(?<![0-9a-f:])(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}(?![0-9a-f:])", re.I)
CONTROLLER_PATH = re.compile(r"(?:/Users/[^/\s]+|/home/[^/\s]+|[A-Za-z]:\\Users\\[^\\\s]+)")
SECRET_VALUE = re.compile(
    r"(?i)(?:password|passwd|api[_-]?key|access[_-]?token|secret[_-]?key|authorization)"
    r"\s*(?:=|:)\s*(?!<redacted>|\*{3,}|null\b|false\b|true\b)[\"']?[^\s,}\"']+"
)


def require_object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError(f"{label} must be an object")
    return value


def require_nonempty_strings(value: dict[str, Any], names: set[str], label: str) -> None:
    missing = sorted(name for name in names if not isinstance(value.get(name), str) or not value[name].strip())
    if missing:
        raise EvidenceError(f"{label} missing non-empty strings: {', '.join(missing)}")


def safe_file(case_dir: Path, relative: Any) -> Path:
    if not isinstance(relative, str) or not relative:
        raise EvidenceError("raw log path must be a non-empty string")
    path = case_dir / relative
    try:
        path.resolve(strict=True).relative_to(case_dir.resolve(strict=True))
    except (OSError, ValueError) as error:
        raise EvidenceError(f"raw log escapes case directory: {relative!r}") from error
    if not path.is_file() or path.is_symlink():
        raise EvidenceError(f"raw log is not a regular file: {relative}")
    return path


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def scan(path: Path) -> None:
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError as error:
        raise EvidenceError(f"artifact is not UTF-8 text: {path}") from error
    for label, pattern in (
        ("IP address", IPV4),
        ("IPv6 address", IPV6),
        ("controller path", CONTROLLER_PATH),
        ("secret value", SECRET_VALUE),
    ):
        match = pattern.search(text)
        if match:
            raise EvidenceError(f"{path} contains {label}: {match.group(0)!r}")


def verify_case(case_dir: Path, expected_case: str) -> None:
    manifest = require_object(load_json(case_dir / "manifest.json"), "manifest")
    if manifest.get("schema") != 1 or manifest.get("case") != expected_case:
        raise EvidenceError(f"manifest identity mismatch in {case_dir}")
    require_nonempty_strings(manifest, {"playbook", "fixture_source_sha256"}, "manifest")
    if not SHA256.fullmatch(manifest["fixture_source_sha256"]):
        raise EvidenceError(f"invalid fixture source hash in {case_dir}")
    binaries = require_object(manifest.get("binaries"), "manifest.binaries")
    require_nonempty_strings(binaries, REQUIRED_BINARIES, "manifest.binaries")
    if any(not SHA256.fullmatch(binaries[name]) for name in REQUIRED_BINARIES):
        raise EvidenceError(f"invalid binary hash in {case_dir}")
    require_nonempty_strings(
        require_object(manifest.get("versions"), "manifest.versions"),
        REQUIRED_VERSIONS,
        "manifest.versions",
    )
    fixture = require_object(manifest.get("fixture"), "manifest.fixture")
    require_nonempty_strings(fixture, {"kind"}, "manifest.fixture")
    if not require_object(fixture.get("specification"), "manifest.fixture.specification"):
        raise EvidenceError(f"fixture specification is empty in {case_dir}")
    correctness = require_object(manifest.get("correctness"), "manifest.correctness")
    bad = sorted(name for name in REQUIRED_CORRECTNESS if correctness.get(name) is not True)
    if bad:
        raise EvidenceError(f"correctness is not proven in {case_dir}: {', '.join(bad)}")

    samples = load_samples(case_dir / "samples.jsonl")
    declared_repetitions = manifest.get("repetitions")
    if (
        not isinstance(declared_repetitions, int)
        or isinstance(declared_repetitions, bool)
        or declared_repetitions < 3
    ):
        raise EvidenceError(f"manifest repetitions must be at least 3 in {case_dir}")
    accepted = {executor: 0 for executor in EXECUTORS}
    seen_repetitions: set[tuple[str, int]] = set()
    orders: dict[int, dict[str, int]] = {}
    for number, sample in enumerate(samples, 1):
        executor = sample.get("executor")
        if executor not in accepted:
            raise EvidenceError(f"sample {number} has invalid executor")
        repetition = sample.get("repetition")
        if not isinstance(repetition, int) or isinstance(repetition, bool) or repetition < 1:
            raise EvidenceError(f"sample {number} has invalid repetition")
        key = (executor, repetition)
        if key in seen_repetitions:
            raise EvidenceError(f"duplicate sample {executor} repetition {repetition}")
        seen_repetitions.add(key)
        order = sample.get("execution_order")
        if order not in {1, 2}:
            raise EvidenceError(f"sample {number} has invalid execution_order")
        orders.setdefault(repetition, {})[executor] = order
        if sample.get("accepted") is True:
            accepted[executor] += 1
        for stream in ("stdout", "stderr"):
            metadata = require_object(sample.get(stream), f"sample {number}.{stream}")
            if not isinstance(metadata.get("sha256"), str) or not SHA256.fullmatch(metadata["sha256"]):
                raise EvidenceError(f"sample {number}.{stream} has invalid hash")
            path = safe_file(case_dir, metadata.get("path"))
            if digest(path) != metadata["sha256"]:
                raise EvidenceError(f"sample {number}.{stream} hash mismatch")
    wrong_counts = sorted(
        executor for executor, count in accepted.items() if count != declared_repetitions
    )
    if wrong_counts:
        if any(accepted[executor] < 3 for executor in wrong_counts):
            raise EvidenceError(
                f"fewer than 3 accepted samples in {case_dir}: "
                f"{', '.join(executor for executor in wrong_counts if accepted[executor] < 3)}"
            )
        raise EvidenceError(
            f"accepted samples do not equal declared repetitions in {case_dir}: "
            f"{', '.join(wrong_counts)}"
        )
    expected_repetitions = set(range(1, declared_repetitions + 1))
    for executor in EXECUTORS:
        actual_repetitions = {
            repetition for sample_executor, repetition in seen_repetitions
            if sample_executor == executor
        }
        if actual_repetitions != expected_repetitions:
            raise EvidenceError(f"{executor} repetition set is incomplete in {case_dir}")
    for repetition, executor_orders in orders.items():
        if executor_orders != {
            "ansible": 1 if repetition % 2 else 2,
            "ruxel": 2 if repetition % 2 else 1,
        }:
            raise EvidenceError(f"execution order is not alternating at repetition {repetition}")
    if expected_case == "six-host":
        gate = require_object(manifest.get("gate_artifacts"), "manifest.gate_artifacts")
        if (
            gate.get("ordered_recaps") is not True
            or gate.get("sshd_leaks") != 0
            or gate.get("unreachable_exit_status") != 1
        ):
            raise EvidenceError("six-host acceptance gates are incomplete")
        for stream in ("stdout", "stderr"):
            path = safe_file(case_dir, f"logs/unreachable.{stream}")
            key = f"unreachable_{stream}_sha256"
            if gate.get(key) != digest(path):
                raise EvidenceError(f"six-host {stream} unreachable log hash mismatch")

    expected_summary = summarize_samples(samples)
    expected_summary["case"] = expected_case
    if load_json(case_dir / "summary.json") != expected_summary:
        raise EvidenceError(f"summary does not match raw samples in {case_dir}")

    for path in case_dir.rglob("*"):
        if path.is_file():
            scan(path)


def verify_root(root: Path) -> None:
    if not root.is_dir():
        raise EvidenceError(f"artifact root does not exist: {root}")
    actual = {path.name for path in root.iterdir() if path.is_dir()}
    if actual != REQUIRED_CASES:
        raise EvidenceError(
            f"case set mismatch: missing={sorted(REQUIRED_CASES - actual)} "
            f"extra={sorted(actual - REQUIRED_CASES)}"
        )
    for case in sorted(REQUIRED_CASES):
        verify_case(root / case, case)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path, nargs="?")
    parser.add_argument(
        "--case",
        nargs=2,
        metavar=("CASE_DIR", "CASE"),
        help="verify one case while an artifact matrix is being assembled",
    )
    args = parser.parse_args()
    if args.case:
        if args.root is not None:
            parser.error("root and --case are mutually exclusive")
        case_dir, case = args.case
        if case not in REQUIRED_CASES:
            parser.error(f"unknown benchmark case: {case}")
        verify_case(Path(case_dir), case)
        print(f"BENCHMARK EVIDENCE PASS: {case}")
        return 0
    if args.root is None:
        parser.error("root or --case is required")
    verify_root(args.root)
    print(f"BENCHMARK EVIDENCE PASS: {len(REQUIRED_CASES)} cases")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except EvidenceError as error:
        raise SystemExit(f"benchmark verification: {error}") from error
