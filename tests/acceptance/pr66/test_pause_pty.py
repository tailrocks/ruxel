import os
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

import pause_pty


class PausePtyTest(unittest.TestCase):
    def test_partial_json_times_out_and_reaps_process(self):
        with tempfile.TemporaryDirectory(prefix='pr66-pause-timeout-') as temp:
            root = Path(temp)
            (root / 'schema').mkdir()
            (root / 'schema/Ruxel.pkl').write_text('synthetic')
            agent = root / 'agent'
            agent.write_text('synthetic')
            pid = root / 'pid'
            cli = root / 'partial-cli'
            cli.write_text('#!' + sys.executable + '\n' +
                           'import os,time\n' +
                           f'open({str(pid)!r}, "w").write(str(os.getpid()))\n' +
                           'os.write(1, b"{")\ntime.sleep(30)\n')
            cli.chmod(0o700)
            started = time.monotonic()
            with patch.object(pause_pty, 'ROOT', root), patch.object(pause_pty, 'PROMPT_TIMEOUT', 0.3), \
                 patch.object(sys, 'argv', ['pause_pty', '--ruxel', str(cli), '--agent', str(agent)]):
                with self.assertRaisesRegex(RuntimeError, 'pause prompt timed out'):
                    pause_pty.main()
            self.assertLess(time.monotonic() - started, 5)
            with self.assertRaises(ProcessLookupError):
                os.kill(int(pid.read_text()), 0)


if __name__ == '__main__':
    unittest.main()
