#!/usr/bin/env python3
from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "tools/benchmarks/run-scale-provider.sh"


class ScaleRunnerSafetyTests(unittest.TestCase):
    def test_rejects_fewer_than_three_repetitions_before_provider_access(self) -> None:
        result = subprocess.run(
            ["bash", str(RUNNER), "/bin/true", "/bin/true", "2"],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 2)
        self.assertIn("must be >=3", result.stderr)

    def test_fixed_synthetic_fixture_and_safety_boundaries(self) -> None:
        source = RUNNER.read_text(encoding="utf-8")
        self.assertIn("tools/fixture-project/benchmarks/scale-65x52.yml", source)
        self.assertIn("--dry-secrets", source)
        self.assertIn("collection-overlay", source)
        self.assertIn("onepassword.py", source)
        self.assertIn("source tools/fixtures/lib.sh", source)
        self.assertIn("trap cleanup EXIT INT TERM", source)
        self.assertIn('resolve_fixture "$fixture"', source)
        self.assertNotIn("ChainArgos", source)


if __name__ == "__main__":
    unittest.main()
