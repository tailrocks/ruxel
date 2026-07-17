import tempfile
import unittest
from pathlib import Path

from validate_scale import validate


FIXTURE = Path(__file__).parents[1] / "fixture-project" / "benchmarks" / "scale-65x52.yml"


class ScaleFixtureTests(unittest.TestCase):
    def test_committed_fixture_has_exact_bounded_scale(self):
        self.assertEqual(validate(FIXTURE), [])

    def test_rejects_wrong_scale_and_real_identity_markers(self):
        text = FIXTURE.read_text()
        damaged = text.replace("    - name: Resolve synthetic benchmark secret 01\n", "", 1)
        damaged = damaged.replace("ruxel-benchmark-item-01", "op://ChainArgos/real/field", 1)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "damaged.yml"
            path.write_text(damaged)
            errors = validate(path)
        self.assertTrue(any("expected 65 tasks" in error for error in errors))
        self.assertTrue(any("item sequence" in error for error in errors))
        self.assertTrue(any("forbidden real identity" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
