#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "tools/benchmarks/run-provider.sh"


class ProviderRunnerSafetyTests(unittest.TestCase):
    def run_local_rejection(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["bash", str(RUNNER), *arguments],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_rejects_non_allowlisted_case_before_provider_access(self) -> None:
        result = self.run_local_rejection("../../private.yml", "/bin/true", "/bin/true", "3")
        self.assertEqual(result.returncode, 2)
        self.assertIn("case must be one of", result.stderr)

    def test_rejects_fewer_than_three_repetitions_before_provider_access(self) -> None:
        result = self.run_local_rejection("fresh", "/bin/true", "/bin/true", "2")
        self.assertEqual(result.returncode, 2)
        self.assertIn("must be >=3", result.stderr)

    def test_structural_fixture_boundary_and_cleanup_are_present(self) -> None:
        source = RUNNER.read_text(encoding="utf-8")
        self.assertIn("source tools/fixtures/lib.sh", source)
        self.assertIn('resolve_fixture "$fixture"', source)
        self.assertIn("trap cleanup EXIT INT TERM", source)
        self.assertIn('tools/fixtures/destroy.sh "$fixture"', source)
        self.assertIn("RUXEL_CAPTURE_BENCH_ELAPSED", source)
        self.assertNotIn("ansible_ssh_host=$", source)

    def test_only_committed_fixture_playbooks_are_selected(self) -> None:
        source = RUNNER.read_text(encoding="utf-8")
        expected = {
            "files-content.yml",
            "performance-snapshots.yml",
            "storage-ext4.yml",
            "postgresql-ownership.yml",
        }
        selected = {
            line.split("playbook=", 1)[1].split(";", 1)[0]
            for line in source.splitlines()
            if ") playbook=" in line
        }
        self.assertEqual(selected, expected)

    def test_monotonic_timer_preserves_command_status_and_streams(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            paths = [Path(temporary) / name for name in ("elapsed", "status", "stdout", "stderr")]
            result = subprocess.run(
                [
                    "python3",
                    str(ROOT / "tools/benchmarks/run_timed.py"),
                    *(str(path) for path in paths),
                    "--",
                    "sh",
                    "-c",
                    "printf output; printf error >&2; exit 7",
                ],
                cwd=ROOT,
                check=False,
            )
            self.assertEqual(result.returncode, 0)
            self.assertGreater(int(paths[0].read_text()), 0)
            self.assertEqual(paths[1].read_text().strip(), "7")
            self.assertEqual(paths[2].read_text(), "output")
            self.assertEqual(paths[3].read_text(), "error")


if __name__ == "__main__":
    unittest.main()
