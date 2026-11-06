import tempfile
import unittest
from pathlib import Path

from validate_scale import validate


FIXTURE = Path(__file__).parents[1] / "fixture-project" / "benchmarks" / "scale-65x52.yml"


class ScaleFixtureTests(unittest.TestCase):
    def test_committed_fixture_has_exact_bounded_scale(self):
        self.assertEqual(validate(FIXTURE), [])

    def test_rejects_wrong_scale_and_item_sequence(self):
        text = FIXTURE.read_text()
        damaged = text.replace("    - name: Resolve synthetic benchmark secret 01\n", "", 1)
        damaged = damaged.replace("ruxel-benchmark-item-01", "ruxel-benchmark-item-invalid", 1)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "damaged.yml"
            path.write_text(damaged)
            errors = validate(path)
        self.assertTrue(any("expected 65 tasks" in error for error in errors))
        self.assertTrue(any("item sequence" in error for error in errors))

    def test_rejects_comment_only_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "damaged.yml"
            path.write_bytes(FIXTURE.read_bytes() + b"\n# harmless change\n")
            errors = validate(path)
        self.assertIn("fixture does not match the approved synthetic fixture", errors)


if __name__ == "__main__":
    unittest.main()
