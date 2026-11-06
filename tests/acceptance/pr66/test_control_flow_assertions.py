import copy
import os
import pwd
import tempfile
import unittest
from pathlib import Path

from control_flow_assertions import assert_observations, assert_retry_attempts


class ControlFlowAssertionsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        (self.root / 'file').write_text('value')
        (self.root / 'marker').write_text('')
        self.observations = [
            {'kind': 'file', 'path': '__RUN_ROOT__/file', 'content': 'value', 'owner': pwd.getpwuid(os.getuid()).pw_name},
            {'kind': 'file_exists', 'path': '__RUN_ROOT__/marker'},
            {'kind': 'task_loop_stdout', 'host': 'fixture', 'task': 'loop', 'values': ['a', 'b']},
            {'kind': 'task_result', 'host': 'fixture', 'task': 'retry', 'field': 'rc', 'equals': 0},
            {'kind': 'fact', 'host': 'fixture', 'name': 'rescued', 'equals': True},
            {'kind': 'output_excludes', 'values': ['synthetic-private']},
        ]
        self.events = [
            {'host': 'fixture', 'task': 'loop', 'result': {'results': [{'stdout': 'a'}, {'stdout': 'b'}]}},
            {'host': 'fixture', 'task': 'retry', 'result': {'rc': 0}},
            {'host': 'fixture', 'task': 'Record rescued synthetic state', 'result': {'facts': {'rescued': True}}},
        ]
        self.outputs = {'stdout': '', 'stderr': '', 'log': ''}

    def check(self):
        assert_observations(self.root, self.observations, self.events, self.outputs)

    def test_complete_observations_and_retry_count(self):
        self.check()
        assert_retry_attempts(2, 2)

    def test_each_observation_fails_closed(self):
        for index in range(len(self.observations)):
            with self.subTest(index=index):
                old = copy.deepcopy(self.observations)
                if index == 0:
                    self.observations[index]['content'] = 'wrong'
                elif index == 1:
                    self.observations[index]['path'] = '__RUN_ROOT__/absent'
                elif index == 2:
                    self.observations[index]['values'].reverse()
                elif index == 3:
                    self.observations[index]['equals'] = 1
                elif index == 4:
                    self.observations[index]['equals'] = False
                else:
                    self.outputs['log'] = 'synthetic-private'
                with self.assertRaises(RuntimeError):
                    self.check()
                self.observations = old
                self.outputs['log'] = ''

    def test_missing_censored_duplicate_and_unknown_evidence_fails(self):
        for bad in ({'censored': True}, {'results': [{'stdout': 'a'}]}, {'results': [{'stdout': 'a'}, {}]}):
            old = self.events[0]['result']
            self.events[0]['result'] = bad
            with self.assertRaises(RuntimeError):
                self.check()
            self.events[0]['result'] = old
        self.events.append(copy.deepcopy(self.events[1]))
        with self.assertRaises(RuntimeError):
            self.check()
        self.events.pop()
        self.observations.append({'kind': 'unknown'})
        with self.assertRaises(RuntimeError):
            self.check()
        for count in (None, 1, True):
            with self.assertRaises(RuntimeError):
                assert_retry_attempts(2, count)

    def test_owner_and_paths_are_checked(self):
        self.observations[0]['owner'] = 'missing-user-pr66-fixture'
        with self.assertRaises(KeyError):
            self.check()
        self.observations[0]['owner'] = pwd.getpwuid(os.getuid()).pw_name
        (self.root / 'file').unlink()
        (self.root / 'file').symlink_to(self.root / 'marker')
        with self.assertRaises(RuntimeError):
            self.check()


if __name__ == '__main__':
    unittest.main()
