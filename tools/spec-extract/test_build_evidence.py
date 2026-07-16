import json
import tempfile
import unittest
from pathlib import Path

from build_evidence import build


class EvidenceTests(unittest.TestCase):
    def test_missing_capture_is_reported_and_existing_task_maps(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            required = root / "required.json"
            fixtures = root / "fixtures.json"
            captures = root / "captures"
            captures.mkdir()
            required.write_text(json.dumps({"features": ["module:copy", "module:file"]}))
            fixtures.write_text(json.dumps({"locations": {
                "module:copy": ["one.yml#Copy"],
                "module:file": ["two.yml#File"],
            }}))
            (captures / "render-parity.jsonl").write_text("")
            (captures / "bless-one.jsonl").write_text(json.dumps({
                "task_name": "Copy", "action": "copy", "status": "ok", "result": {}
            }) + "\n")

            evidence = build(required, fixtures, captures)
            self.assertIn("module:copy", evidence["entries"])
            self.assertEqual(evidence["missing"], ["module:file"])


if __name__ == "__main__":
    unittest.main()
