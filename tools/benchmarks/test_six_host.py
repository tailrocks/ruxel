import hashlib
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from artifact import append_sample, sanitize


class SixHostArtifactTests(unittest.TestCase):
    def test_runner_preserves_acceptance_gates_and_alternating_order(self):
        runner = Path(__file__).parent / "run-six-host.sh"
        subprocess.run(["bash", "-n", runner], check=True)
        text = runner.read_text()
        ansible = text.index('timed_sample ansible "$repetition"')
        ruxel = text.index('timed_sample ruxel "$repetition"')
        self.assertLess(ansible, ruxel)
        for requirement in (
            "ordered recaps",
            "leaked sshd children",
            "unreachable_status",
            "containers were not reaped",
            'python3 tools/benchmarks/verify.py --case "$case_dir" six-host',
        ):
            self.assertIn(requirement, text)

    def test_sanitize_removes_fixture_address_and_controller_home(self):
        cleaned = sanitize("target 192.0.2.7 cwd=/Users/operator/project\n")
        self.assertEqual(cleaned, "target <fixture-ip> cwd=<controller-home>/project\n")

    def test_append_sample_sanitizes_then_hashes_both_streams(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            logs = root / "logs"
            logs.mkdir()
            stdout = logs / "ansible-1.stdout"
            stderr = logs / "ansible-1.stderr"
            stdout.write_text("connected 198.51.100.8\n")
            stderr.write_text("cwd /home/operator/ruxel\n")
            append_sample(root, "ansible", 1, 1, 123, stdout, stderr)
            sample = json.loads((root / "samples.jsonl").read_text())
            self.assertEqual(sample["elapsed_ns"], 123)
            self.assertEqual(sample["execution_order"], 1)
            self.assertNotIn("198.51.100.8", stdout.read_text())
            self.assertNotIn("/home/operator", stderr.read_text())
            self.assertEqual(
                sample["stdout"]["sha256"],
                hashlib.sha256(stdout.read_bytes()).hexdigest(),
            )


if __name__ == "__main__":
    unittest.main()
