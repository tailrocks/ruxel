import json
import tempfile
import unittest
from pathlib import Path

from normalize_capture import normalize
from verify_captures import required_parity_stems, walk


class CaptureSchemaTests(unittest.TestCase):
    def test_normalizer_removes_run_specific_nested_fields(self):
        record = normalize({
            "task_name": "Pause", "action": "pause", "status": "ok",
            "raw_args": {},
            "result": {"changed": False, "start": "time", "stop": "time", "delta": "1"},
        })
        self.assertEqual(record["result"], {"changed": False})

    def test_verifier_rejects_controller_paths_and_uuid_values(self):
        with self.assertRaises(ValueError):
            walk({"value": "/Users/operator/private"})
        with self.assertRaises(ValueError):
            walk({"value": "550e8400-e29b-41d4-a716-446655440000"})

    def test_parity_matrix_requires_every_declared_playbook(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "matrix.json"
            path.write_text(json.dumps([
                {"playbook": "alpha.yml"},
                {"playbook": "nested.name.yaml"},
            ]))
            self.assertEqual(required_parity_stems(path), {"alpha", "nested.name"})


if __name__ == "__main__":
    unittest.main()
