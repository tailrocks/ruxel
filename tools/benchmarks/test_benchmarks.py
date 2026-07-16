#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import summarize
import verify


class BenchmarkEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for case in verify.REQUIRED_CASES:
            self.make_case(case)

    def tearDown(self) -> None:
        self.temp.cleanup()

    @staticmethod
    def write_json(path: Path, value: object) -> None:
        path.write_text(json.dumps(value, sort_keys=True) + "\n", encoding="utf-8")

    def make_case(self, case: str) -> None:
        directory = self.root / case
        logs = directory / "logs"
        logs.mkdir(parents=True)
        manifest = {
            "schema": 1,
            "case": case,
            "playbook": f"{case}.yml",
            "fixture_source_sha256": "a" * 64,
            "binaries": {"controller_sha256": "b" * 64, "agent_sha256": "c" * 64},
            "versions": {
                "ansible": "2.21.2",
                "ruxel": "0.1.0",
                "agent": "0.1.0",
                "rustc": "1.88.0",
                "os": "Debian 12",
                "kernel": "6.1",
            },
            "fixture": {
                "kind": "disposable-provider-twin",
                "specification": {"image": "debian-12", "server_type": "fixture"},
            },
            "repetitions": 3,
            "correctness": {name: True for name in verify.REQUIRED_CORRECTNESS},
        }
        if case == "six-host":
            gate_hashes = {}
            for stream in ("stdout", "stderr"):
                relative = f"logs/unreachable.{stream}"
                data = f"unreachable {stream}\n".encode()
                (directory / relative).write_bytes(data)
                gate_hashes[f"unreachable_{stream}_sha256"] = hashlib.sha256(data).hexdigest()
            manifest["gate_artifacts"] = {
                "ordered_recaps": True,
                "sshd_leaks": 0,
                "unreachable_exit_status": 1,
                **gate_hashes,
            }
        self.write_json(directory / "manifest.json", manifest)
        samples = []
        for executor, base in (("ansible", 20), ("ruxel", 10)):
            for repetition in range(1, 4):
                sample = {
                    "executor": executor,
                    "repetition": repetition,
                    "execution_order": (
                        (1 if repetition % 2 else 2)
                        if executor == "ansible"
                        else (2 if repetition % 2 else 1)
                    ),
                    "accepted": True,
                    "elapsed_ns": base + repetition,
                }
                for stream in ("stdout", "stderr"):
                    relative = f"logs/{executor}-{repetition}.{stream}"
                    data = f"{executor} {repetition} {stream}\n".encode()
                    (directory / relative).write_bytes(data)
                    sample[stream] = {
                        "path": relative,
                        "sha256": hashlib.sha256(data).hexdigest(),
                    }
                samples.append(sample)
        (directory / "samples.jsonl").write_text(
            "".join(json.dumps(sample, sort_keys=True) + "\n" for sample in samples),
            encoding="utf-8",
        )
        summary = summarize.summarize_samples(samples)
        summary["case"] = case
        self.write_json(directory / "summary.json", summary)

    def test_complete_root_passes(self) -> None:
        verify.verify_root(self.root)

    def test_single_case_passes(self) -> None:
        verify.verify_case(self.root / "six-host", "six-host")

    def test_statistics_are_recomputed(self) -> None:
        summary = summarize.summarize_samples(
            [
                {"executor": "ansible", "accepted": True, "elapsed_ns": value}
                for value in (10, 20, 100)
            ]
            + [
                {"executor": "ruxel", "accepted": True, "elapsed_ns": value}
                for value in (5, 10, 50)
            ]
        )
        self.assertEqual(summary["executors"]["ansible"]["median_ns"], 20)
        self.assertEqual(summary["executors"]["ansible"]["p95_ns"], 100)
        self.assertEqual(summary["speedup"], 2.0)

    def test_missing_case_fails(self) -> None:
        (self.root / "fresh").rename(self.root / "fresh-missing")
        with self.assertRaisesRegex(summarize.EvidenceError, "case set mismatch"):
            verify.verify_root(self.root)

    def test_too_few_accepted_samples_fails(self) -> None:
        path = self.root / "fresh" / "samples.jsonl"
        samples = summarize.load_samples(path)
        samples[0]["accepted"] = False
        path.write_text("".join(json.dumps(value) + "\n" for value in samples), encoding="utf-8")
        with self.assertRaisesRegex(summarize.EvidenceError, "fewer than 3"):
            verify.verify_case(self.root / "fresh", "fresh")

    def test_raw_log_hash_mismatch_fails(self) -> None:
        (self.root / "fresh" / "logs" / "ansible-1.stdout").write_text("changed\n")
        with self.assertRaisesRegex(summarize.EvidenceError, "hash mismatch"):
            verify.verify_case(self.root / "fresh", "fresh")

    def test_secret_ip_and_controller_path_fail(self) -> None:
        log = self.root / "fresh" / "logs" / "ansible-1.stdout"
        original = log.read_bytes()
        for unsafe, label in (
            ("password=visible-value\n", "secret value"),
            ("connected to 192.0.2.1\n", "IP address"),
            ("cwd=/Users/operator/project\n", "controller path"),
        ):
            log.write_text(unsafe)
            with self.assertRaisesRegex(summarize.EvidenceError, label):
                verify.scan(log)
        log.write_bytes(original)

    def test_stale_summary_fails(self) -> None:
        summary_path = self.root / "fresh" / "summary.json"
        summary = summarize.load_json(summary_path)
        summary["speedup"] = 999
        self.write_json(summary_path, summary)
        with self.assertRaisesRegex(summarize.EvidenceError, "summary does not match"):
            verify.verify_case(self.root / "fresh", "fresh")


if __name__ == "__main__":
    unittest.main()
