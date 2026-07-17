import json
import tempfile
import unittest
from pathlib import Path

from compare_diffs import ansible_diffs, ruxel_diffs


class DiffParityTests(unittest.TestCase):
    def test_stdout_and_json_whole_file_diffs_match(self):
        stdout = """TASK [Synthetic] ****
--- before: /tmp/file
+++ after: /tmp/file
@@ -1 +1 @@
-old
+new
changed: [fixture]
"""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ruxel.jsonl"
            path.write_text(json.dumps({
                "event": "task", "task": "Synthetic",
                "result": {"diff": "--- before\n+++ after\n-old\n+new\n"},
            }) + "\n")
            self.assertEqual(ruxel_diffs(path), ansible_diffs(stdout))

    def test_different_line_order_does_not_match(self):
        self.assertNotEqual(
            ansible_diffs("TASK [T] ****\n--- a\n+++ b\n-a\n-b\n+c\n"),
            ansible_diffs("TASK [T] ****\n--- a\n+++ b\n-b\n-a\n+c\n"),
        )

    def test_full_snapshot_matches_ansible_added_line_delta(self):
        stdout = "TASK [T] ****\n--- a\n+++ b\n+new\n"
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ruxel.jsonl"
            path.write_text(json.dumps({
                "event": "task", "task": "T",
                "result": {"diff": "--- a\n+++ b\n old\n-old\n+old\n+new\n"},
            }) + "\n")
            self.assertEqual(ruxel_diffs(path), ansible_diffs(stdout))


if __name__ == "__main__":
    unittest.main()
