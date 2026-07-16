import json
import tempfile
import unittest
from pathlib import Path

from compare_results import compare, normalize_diff, normalize_value


class ResultParityTests(unittest.TestCase):
    def test_normalization_keeps_only_observable_registered_fields(self):
        self.assertEqual(
            normalize_value({
                "changed": False,
                "invocation": {"module_args": {"secret": "hidden"}},
                "delta": "0:00:01",
                "stat": {"exists": True, "mode": "0644", "isdir": False},
            }, "stat"),
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

    def test_metadata_only_diff_is_ignored_after_normalization(self):
        self.assertEqual(
            normalize_value({
                "changed": True,
                "diff": [{"before_header": "old"}, {"after_header": "new"}],
            }, "lineinfile"),
            {"changed": True},
        )
        self.assertEqual(
            normalize_value({"changed": True, "diff": [{"before": "", "after": ""}]}),
            {"changed": True},
        )

    def test_structured_and_unified_content_diffs_match(self):
        structured = [{"before": "old\n", "after": "new\n",
                       "before_header": "/tmp/file", "after_header": "/tmp/file"}]
        unified = "--- before\n+++ after\n-old\n+new\n"
        self.assertEqual(normalize_diff(structured), normalize_diff(unified))

    def test_diff_normalization_preserves_order_and_unchanged_context(self):
        self.assertNotEqual(
            normalize_diff([{"before": "a\nb", "after": "c\nb"}]),
            normalize_diff([{"before": "b\na", "after": "b\nc"}]),
        )

    def test_redaction_is_observable(self):
        self.assertEqual(normalize_value({"changed": True, "censored": "hidden"}),
                         {"changed": True, "redacted": True})


if __name__ == "__main__":
    unittest.main()
