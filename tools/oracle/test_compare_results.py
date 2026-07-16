import json
import tempfile
import unittest
from pathlib import Path

from compare_results import compare, normalize_value


class ResultParityTests(unittest.TestCase):
    def test_normalization_keeps_only_observable_registered_fields(self):
        self.assertEqual(
            normalize_value({
                "changed": False,
                "invocation": {"module_args": {"secret": "hidden"}},
                "delta": "0:00:01",
                "stat": {"exists": True, "mode": "0644", "isdir": False},
            }),
            {"changed": False, "stat": {"exists": True, "isdir": False}},
        )

    def test_equivalent_events_match_across_formats(self):
        with tempfile.TemporaryDirectory() as directory:
            ruxel = Path(directory) / "ruxel.jsonl"
            ansible = Path(directory) / "ansible.jsonl"
            ruxel.write_text(json.dumps({
                "event": "task", "task": "Synthetic", "module": "command",
                "status": "changed", "ignored": False,
                "result": {"changed": True, "failed": False, "rc": 0,
                           "stdout": "ok", "internal": "ignored"},
            }) + "\n")
            ansible.write_text(json.dumps({
                "task_name": "Play : Synthetic", "action": "ansible.legacy.command",
                "status": "ok", "result": {"changed": True, "failed": False,
                                            "rc": 0, "stdout": "ok",
                                            "delta": "variable"},
            }) + "\n")
            self.assertEqual(compare(ruxel, ansible), 0)


if __name__ == "__main__":
    unittest.main()
