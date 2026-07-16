import unittest

from normalize_capture import normalize
from verify_captures import walk


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


if __name__ == "__main__":
    unittest.main()
