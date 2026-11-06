#!/usr/bin/env python3
"""Synthetic SSH/SFTP endpoint; never starts a network client or remote command."""
import hashlib
import json
import os
from pathlib import Path
import struct
import sys

ROOT = Path(__file__).absolute().parents[1]
LOG = ROOT / 'calls.jsonl'
ARGV = sys.argv[1:]
SEPARATOR = ARGV.index('--')
assert ARGV[SEPARATOR + 1] == 'fixture.invalid'
OPTIONS = ARGV[:SEPARATOR]
assert 'StrictHostKeyChecking=yes' in OPTIONS
assert 'IdentitiesOnly=yes' in OPTIONS and OPTIONS[OPTIONS.index('-p') + 1] == '2222'
assert not set(OPTIONS) & {'-F', '-M', '-S', '-O', '-A', '-J'}
assert str(ROOT / 'identity key') in OPTIONS
assert 'UserKnownHostsFile=' + str(ROOT / 'known hosts') in OPTIONS
REMOTE = ARGV[SEPARATOR + 2:]


def record(event):
    with LOG.open('a') as output:
        output.write(json.dumps(event) + '\n')


def local(path):
    assert path.startswith('/var/lib/ruxel') and '..' not in Path(path).parts
    return ROOT / 'remote' / path.removeprefix('/')


def string(value):
    if isinstance(value, str):
        value = value.encode()
    return struct.pack('>I', len(value)) + value


def take(data):
    size = struct.unpack('>I', data[:4])[0]
    return data[4:4 + size], data[4 + size:]


def exact(size):
    data = sys.stdin.buffer.read(size)
    assert len(data) == size
    return data


def packet(data):
    sys.stdout.buffer.write(struct.pack('>I', len(data)) + data)
    sys.stdout.buffer.flush()


def sftp():
    handles = {}
    while prefix := sys.stdin.buffer.read(4):
        data = exact(struct.unpack('>I', prefix)[0])
        kind, data = data[0], data[1:]
        if kind == 1:
            packet(b'\x02' + struct.pack('>I', 3) + string('posix-rename@openssh.com') + string('1'))
            continue
        request, data = data[:4], data[4:]
        if kind == 3:
            path, data = take(data)
            flags = struct.unpack('>I', data[:4])[0]
            assert flags & 0x2 and flags & 0x8 and flags & 0x20
            handle = str(len(handles)).encode()
            handles[handle] = local(path.decode()).open('xb')
            record(['open', path.decode(), flags])
            packet(b'\x66' + request + string(handle))
            continue
        if kind == 6:
            handle, data = take(data)
            offset = struct.unpack('>Q', data[:8])[0]
            content, rest = take(data[8:])
            assert not rest
            if (ROOT / 'sftp-fail').exists():
                packet(b'\x65' + request + struct.pack('>I', 4) + string('synthetic write failure') + string(''))
                continue
            handles[handle].seek(offset)
            handles[handle].write(content)
        elif kind == 4:
            handle, rest = take(data)
            assert not rest
            handles[handle].close()
        elif kind in (18, 200):
            if kind == 200:
                extension, data = take(data)
                assert extension == b'posix-rename@openssh.com'
            source, data = take(data)
            target, rest = take(data)
            assert not rest
            os.rename(local(source.decode()), local(target.decode()))
            record(['rename', source.decode(), target.decode()])
        else:
            raise AssertionError(('unsupported SFTP request', kind))
        packet(b'\x65' + request + struct.pack('>I', 0) + string('') + string(''))


def frame():
    size, shift = 0, 0
    while True:
        byte = exact(1)[0]
        size |= (byte & 127) << shift
        if not byte & 128:
            break
        shift += 7
    return exact(size)


record(['ssh', ARGV])
if '-s' in OPTIONS:
    assert REMOTE == ['sftp']
    sftp()
elif REMOTE[:3] == ['test', '!', '-L']:
    sys.exit(local(REMOTE[3]).is_symlink())
elif REMOTE[:2] == ['test', '-x']:
    sys.exit(not os.access(local(REMOTE[2]), os.X_OK))
elif REMOTE[:2] == ['mkdir', '-p']:
    local(REMOTE[2]).mkdir(parents=True, exist_ok=True)
elif REMOTE[0] == 'chmod':
    os.chmod(local(REMOTE[2]), int(REMOTE[1], 8))
elif REMOTE[0] == 'sha256sum':
    digest = '0' * 64 if (ROOT / 'mismatch').exists() else hashlib.sha256(local(REMOTE[1]).read_bytes()).hexdigest()
    print(digest + '  ' + REMOTE[1])
elif REMOTE[:2] == ['rm', '-f']:
    if (ROOT / 'cleanup-fail').exists():
        sys.exit(23)
    local(REMOTE[2]).unlink(missing_ok=True)
else:
    assert len(REMOTE) == 1 and REMOTE[0].startswith('/var/lib/ruxel/agent/')
    record(['launch', REMOTE[0]])
    (ROOT / 'hello').write_bytes(frame())
    sys.stdout.buffer.write((ROOT / 'ack').read_bytes())
    sys.stdout.buffer.flush()
    (ROOT / 'done').write_bytes(frame())
