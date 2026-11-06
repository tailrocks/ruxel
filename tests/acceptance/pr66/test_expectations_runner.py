import contextlib
import copy
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import expectations_runner
from expectations_runner import path_in, validate_case


class ExpectationsTest(unittest.TestCase):
    def test_rejects_missing_or_unknown_assertions(self):
        case = {
            'fixture': 'check-semantics.pkl',
            'observations': [{'kind': 'file', 'path': '__RUN_ROOT__/x', 'content': 'ok'}],
            'drift': [{'kind': 'write_file', 'path': '__RUN_ROOT__/x', 'content': 'bad'}],
            'phases': {
                'fresh': {'observations_match': True, 'changed': True},
                'converged': {'observations_match': True, 'changed': False},
                'drift_repair': {'observations_match': True, 'changed': True},
                'check': {'state_unchanged': True},
                'diff': {'state_unchanged': True},
            },
        }
        validate_case('check-semantics', case)
        missing = copy.deepcopy(case)
        del missing['phases']['fresh']['observations_match']
        with self.assertRaises(RuntimeError):
            validate_case('check-semantics', missing)
        unknown = copy.deepcopy(case)
        unknown['phases']['check']['unverified'] = True
        with self.assertRaises(RuntimeError):
            validate_case('check-semantics', unknown)
        with self.assertRaises(RuntimeError):
            validate_case('storage-ext4', case)

    def test_default_all_and_unsupported_selection_fail_without_remote_execution(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            binary = root / 'binary'
            binary.write_text('synthetic')
            (root / 'expectations.json').write_text(json.dumps({
                'version': 1, 'scenarios': {name: {} for name in expectations_runner.SCENARIOS},
            }))
            original = expectations_runner.execute
            def execute_safe(name, case, cli, agent):
                if name in expectations_runner.PINS:
                    return None
                return original(name, case, cli, agent)
            args = ['expectations_runner', '--ruxel', str(binary), '--agent', str(binary)]
            with patch.object(expectations_runner, 'WORKLOADS', root), \
                 patch.object(expectations_runner, 'execute', side_effect=execute_safe), \
                 patch.object(expectations_runner.subprocess, 'run') as launched, \
                 patch('sys.argv', args), contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertTrue(expectations_runner.main())
                self.assertIn('2/14 selected scenarios passed', output.getvalue())
                launched.assert_not_called()
            with patch.object(expectations_runner, 'WORKLOADS', root), \
                 patch.object(expectations_runner.subprocess, 'run') as launched, \
                 patch('sys.argv', args + ['--scenario', 'storage-ext4']), \
                 contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertTrue(expectations_runner.main())
                self.assertIn('no reviewed safe local adapter', output.getvalue())
                launched.assert_not_called()

    def test_paths_stay_under_temporary_root(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'payload'
            root.mkdir()
            self.assertEqual(path_in(root, '__RUN_ROOT__/ok'), root / 'ok')
            for value in ('/etc/passwd', '__RUN_ROOT__/../escape', '__RUN_ROOT__//etc/passwd'):
                with self.assertRaises(RuntimeError):
                    path_in(root, value)
            (root / 'link').symlink_to(Path(temp), target_is_directory=True)
            with self.assertRaises(RuntimeError):
                path_in(root, '__RUN_ROOT__/link/outside')


if __name__ == '__main__':
    unittest.main()
