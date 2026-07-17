#!/usr/bin/env python3
import unittest

from normalize_capture import normalize


class NormalizeCaptureTests(unittest.TestCase):
    def test_drops_volatile_module_diagnostics(self):
        record = normalize({
            "task_name": "restart",
            "action": "service",
            "status": "ok",
            "result": {
                "changed": True,
                "src": "/root/.ansible/tmp/ansible-tmp-123/.source",
                "status": {"ActiveEnterTimestamp": "today", "ExecMainPID": "42"},
            },
            "raw_args": {"name": "ssh", "state": "restarted"},
        })
        self.assertEqual(record["result"], {"changed": True})

    def test_stat_keeps_only_compatibility_fields(self):
        record = normalize({
            "task_name": "inspect",
            "action": "stat",
            "status": "ok",
            "result": {"changed": False, "stat": {
                "exists": True, "isdir": True, "path": "/tmp/fixture",
                "atime": 123.5, "inode": 99,
            }},
        })
        self.assertEqual(record["result"], {
            "changed": False,
            "stat": {"exists": True, "isdir": True, "path": "/tmp/fixture"},
        })


if __name__ == "__main__":
    unittest.main()
