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
        fixture = Path(__file__).parents[2] / "tools/fixture-project/benchmarks/files.yml"
        parity_path = Path(__file__).parents[2] / "tools/oracle/parity/files-content.json"
        parity = json.loads(parity_path.read_text())
        manifest = {
            "schema": 1,
            "case": case,
            "playbook": "tools/fixture-project/benchmarks/files.yml",
            "fixture_source_sha256": hashlib.sha256(fixture.read_bytes()).hexdigest(),
            "binaries": parity["binaries"],
            "versions": {
                "ansible": "ansible-playbook [core 2.21.2]",
                "ruxel": "0.1.0",
                "agent": "0.1.0",
                "rustc": "rustc 1.97.1 (test fixture)",
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
        if case != "scale-65x52":
            manifest["parity_manifest"] = "tools/oracle/parity/files-content.json"
            manifest["parity_manifest_sha256"] = hashlib.sha256(
                parity_path.read_bytes()
            ).hexdigest()
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
        if case == "scale-65x52":
            manifest["scale_gate"] = {
                "task_count": 65,
                "synthetic_lookup_count": 52,
                "dry_secrets": True,
                "ruxel_median_limit_ns": 5_000_000_000,
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
                    if case == "scale-65x52" and stream == "stdout":
                        if executor == "ruxel":
                            data = (
                                b'{"event":"task","task":"T","module":"assert",'
                                b'"status":"ok","ignored":false,"result":{"changed":false}}\n'
                                b'{"event":"recap","host":"fixture","ok":1,"changed":0,'
                                b'"failed":0,"unreachable":0,"skipped":0,"rescued":0,"ignored":0}\n'
                            )
                        else:
                            data = (
                                b'{"task_name":"T","action":"assert","status":"ok",'
                                b'"result":{"changed":false},"ignore_errors":false,"raw_args":{}}\n'
                            )
                    (directory / relative).write_bytes(data)
                    sample[stream] = {
                        "path": relative,
                        "sha256": hashlib.sha256(data).hexdigest(),
                    }
                samples.append(sample)
        if case == "scale-65x52":
            correctness = directory / "correctness"
            correctness.mkdir()
            for repetition in range(1, 4):
                (correctness / f"ansible-{repetition}.jsonl").write_bytes(
                    (logs / f"ansible-{repetition}.stdout").read_bytes()
                )
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

    def test_fixture_source_hash_mismatch_fails(self) -> None:
        path = self.root / "fresh" / "manifest.json"
        manifest = summarize.load_json(path)
        manifest["fixture_source_sha256"] = "a" * 64
        self.write_json(path, manifest)
        with self.assertRaisesRegex(summarize.EvidenceError, "fixture source hash mismatch"):
            verify.verify_case(self.root / "fresh", "fresh")

    def test_parity_manifest_hash_mismatch_fails(self) -> None:
        path = self.root / "fresh" / "manifest.json"
        manifest = summarize.load_json(path)
        manifest["parity_manifest_sha256"] = "a" * 64
        self.write_json(path, manifest)
        with self.assertRaisesRegex(summarize.EvidenceError, "parity manifest hash mismatch"):
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

    def test_scale_median_limit_is_enforced(self) -> None:
        path = self.root / "scale-65x52" / "samples.jsonl"
        samples = summarize.load_samples(path)
        for sample in samples:
            if sample["executor"] == "ruxel":
                sample["elapsed_ns"] = 5_000_000_000
        path.write_text("".join(json.dumps(value) + "\n" for value in samples), encoding="utf-8")
        summary = summarize.summarize_samples(samples)
        summary["case"] = "scale-65x52"
        self.write_json(self.root / "scale-65x52" / "summary.json", summary)
        with self.assertRaisesRegex(summarize.EvidenceError, "not below 5 seconds"):
            verify.verify_case(self.root / "scale-65x52", "scale-65x52")


if __name__ == "__main__":
    unittest.main()
