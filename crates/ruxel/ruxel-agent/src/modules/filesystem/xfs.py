"""FD-only Linux x86_64/aarch64 XFS data growth helper.

Run with python3 -I -S -c SCRIPT MODE BYTES EXPECTED_RDEV, mount directory on fd0.
The caller holds the matching block device and mount directory throughout.
Exclusive administrative ownership of filesystem resizing is required.
"""
import fcntl
import json
import os
import platform
import stat
import struct
import sys

GEOMETRY = 0x80705864
GROW = 0x4010586e


def geometry(ioctl):
    data = bytearray(112)
    ioctl(0, GEOMETRY, data, True)
    blocksize = struct.unpack_from('=I', data, 0)[0]
    agblocks = struct.unpack_from('=I', data, 8)[0]
    imaxpct = struct.unpack_from('=I', data, 28)[0]
    blocks, rtblocks, rtextents, logstart = struct.unpack_from('=4Q', data, 32)
    if blocksize < 512 or blocksize & (blocksize - 1) or agblocks < 64 or not blocks or imaxpct > 100:
        raise ValueError('invalid XFS geometry')
    if rtblocks or rtextents or not logstart:
        raise ValueError('XFS realtime or external-log layout unsupported')
    return blocksize, imaxpct, blocks, agblocks


def execute(mode, target_bytes, expected_rdev, ioctl=fcntl.ioctl, fstat=os.fstat):
    if sys.version_info < (3, 9):
        raise ValueError('XFS resize requires Python 3.9 or newer')
    if sys.platform != 'linux' or platform.machine() not in ('x86_64', 'aarch64'):
        raise ValueError('unsupported XFS ioctl ABI')
    if mode not in ('preflight', 'check', 'grow') or target_bytes < 0:
        raise ValueError('invalid helper arguments')
    identity = fstat(0)
    if not stat.S_ISDIR(identity.st_mode) or identity.st_dev != expected_rdev:
        raise ValueError('mount descriptor identity mismatch')
    blocksize, imaxpct, before, agblocks = geometry(ioctl)
    if mode == 'preflight':
        return {'changed': False, 'blocks': before, 'blocksize': blocksize}
    target = target_bytes // blocksize
    groups, remainder = divmod(target, agblocks)
    if groups >= (1 << 32) - 1:
        raise ValueError('XFS allocation group limit exceeded')
    if remainder and remainder < 64:
        target -= remainder
    if target < before:
        raise ValueError('refusing XFS shrink')
    if target > (1 << 64) - 1:
        raise ValueError('XFS target overflows u64')
    if mode == 'check' or target == before:
        return {'changed': target > before, 'blocks': before, 'blocksize': blocksize}
    ioctl(0, GROW, struct.pack('=QI4x', target, imaxpct))
    after_blocksize, after_imaxpct, after, after_agblocks = geometry(ioctl)
    if after_blocksize != blocksize or after_imaxpct != imaxpct or after_agblocks != agblocks or after != target:
        raise ValueError('XFS growth postcondition failed')
    return {'changed': after > before, 'blocks': after, 'blocksize': blocksize}


if __name__ == '__main__':
    try:
        print(json.dumps(execute(sys.argv[1], int(sys.argv[2]), int(sys.argv[3]))))
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
