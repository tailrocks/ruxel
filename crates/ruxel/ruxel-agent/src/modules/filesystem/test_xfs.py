import errno
import importlib.util
from pathlib import Path
import stat
import struct
import sys
import types
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('helper', Path(__file__).with_name('xfs.py'))
h = importlib.util.module_from_spec(spec)
spec.loader.exec_module(h)


class Kernel:
    def __init__(self):
        self.blocks = 100
        self.logstart = 64
        self.rtblocks = 0
        self.calls = []
        self.failure = None
        self.ignore_growth = False

    def ioctl(self, fd, request, data, *args):
        self.calls.append((fd, request, bytes(data)))
        if self.failure is not None:
            raise OSError(self.failure, 'synthetic failure')
        if request == h.GEOMETRY:
            assert len(data) == 112
            struct.pack_into('=I', data, 0, 4096)
            struct.pack_into('=I', data, 8, 1024)
            struct.pack_into('=I', data, 28, 25)
            struct.pack_into('=4Q', data, 32, self.blocks, self.rtblocks, 0, self.logstart)
        else:
            assert request == h.GROW and len(data) == 16
            blocks, imaxpct = struct.unpack('=QI4x', data)
            assert imaxpct == 25
            if not self.ignore_growth:
                self.blocks = blocks


class Tests(unittest.TestCase):
    def setUp(self):
        self.kernel = Kernel()
        self.platform = patch.object(h.sys, 'platform', 'linux')
        self.machine = patch.object(h.platform, 'machine', return_value='x86_64')
        self.platform.start()
        self.machine.start()
        self.addCleanup(self.platform.stop)
        self.addCleanup(self.machine.stop)

    def run_helper(self, mode, target=200 * 4096, identity=77):
        return h.execute(mode, target, 77, self.kernel.ioctl,
                         lambda fd: types.SimpleNamespace(st_mode=stat.S_IFDIR, st_dev=identity))

    def test_preflight_only_reads(self):
        self.assertFalse(self.run_helper('preflight')['changed'])
        self.assertEqual([c[1] for c in self.kernel.calls], [h.GEOMETRY])

    def test_check_only_reads(self):
        self.assertTrue(self.run_helper('check')['changed'])
        self.assertEqual([c[1] for c in self.kernel.calls], [h.GEOMETRY])

    def test_growth_and_idempotence_same_fd(self):
        self.assertTrue(self.run_helper('grow')['changed'])
        self.assertFalse(self.run_helper('grow')['changed'])
        self.assertEqual([c[1] for c in self.kernel.calls],
                         [h.GEOMETRY, h.GROW, h.GEOMETRY, h.GEOMETRY])
        self.assertTrue(all(c[0] == 0 for c in self.kernel.calls))

    def test_rounds_small_final_allocation_group_and_converges(self):
        self.assertTrue(self.run_helper('check', 1034 * 4096)['changed'])
        self.assertTrue(self.run_helper('grow', 1034 * 4096)['changed'])
        self.assertEqual(self.kernel.blocks, 1024)
        self.assertFalse(self.run_helper('check', 1034 * 4096)['changed'])
        self.assertFalse(self.run_helper('grow', 1034 * 4096)['changed'])
        self.assertTrue(self.run_helper('check', 1088 * 4096)['changed'])

    def test_shrink_refused(self):
        with self.assertRaisesRegex(ValueError, 'shrink'):
            self.run_helper('grow', 99 * 4096)
        self.assertEqual(len(self.kernel.calls), 1)

    def test_identity_refused_before_ioctl(self):
        with self.assertRaisesRegex(ValueError, 'identity'):
            self.run_helper('grow', identity=88)
        self.assertEqual(self.kernel.calls, [])

    def test_unsupported_layouts(self):
        for field in ('rtblocks', 'logstart'):
            with self.subTest(field=field):
                self.kernel = Kernel()
                setattr(self.kernel, field, 1 if field == 'rtblocks' else 0)
                with self.assertRaisesRegex(ValueError, 'layout'):
                    self.run_helper('preflight')
                self.assertEqual(len(self.kernel.calls), 1)

    def test_kernel_errors_propagate(self):
        for code in (errno.ENOTTY, errno.EPERM, errno.EWOULDBLOCK, errno.EIO):
            with self.subTest(code=code):
                self.kernel.failure = code
                with self.assertRaises(OSError) as caught:
                    self.run_helper('grow')
                self.assertEqual(caught.exception.errno, code)

    def test_postcondition_required(self):
        self.kernel.ignore_growth = True
        with self.assertRaisesRegex(ValueError, 'postcondition'):
            self.run_helper('grow')


unittest.main()
