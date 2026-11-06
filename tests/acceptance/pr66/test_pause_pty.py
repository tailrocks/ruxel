import os
import select
import signal
import subprocess
import sys
import time
import unittest

import pause_pty


class PausePtyTest(unittest.TestCase):
    def test_partial_json_times_out_after_byte_arrives(self):
        process = subprocess.Popen(
            [sys.executable, '-c', 'import os,time; os.write(1, b"{"); time.sleep(30)'],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True,
        )
        try:
            ready, _, _ = select.select([process.stdout], [], [], 10)
            self.assertTrue(ready, 'fake process did not produce its partial JSON byte')
            started = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, 'pause prompt timed out'):
                pause_pty.read_prompt(process.stdout, 0.3)
            self.assertLess(time.monotonic() - started, 5)
            self.assertIsNone(process.poll(), 'fake process exited before the timeout')
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=5)
            process.stdout.close()
            process.stderr.close()
        with self.assertRaises(ProcessLookupError):
            os.kill(process.pid, 0)


if __name__ == '__main__':
    unittest.main()
