#!/usr/bin/env python3
"""OpenSSH shim that cuts selected controller/agent frames deterministically."""

import os
import subprocess
import sys
import threading
import time
from pathlib import Path

LARGE_FRAME_BYTES = 1024 * 1024


def is_agent_command(argv):
    try:
        marker = argv.index("--")
    except ValueError:
        return False
    return marker + 1 < len(argv) and argv[marker + 1].startswith("/var/lib/ruxel/agent/")


def is_sftp_command(argv):
    return "-s" in argv and "sftp" in argv


def read_frame(stream):
    prefix = bytearray()
    shift = 0
    length = 0
    while True:
        byte = stream.read(1)
        if not byte:
            return None if not prefix else bytes(prefix)
        prefix += byte
        value = byte[0]
        length |= (value & 0x7F) << shift
        if value & 0x80 == 0:
            break
        shift += 7
        if shift >= 64:
            return bytes(prefix)
    body = bytearray()
    while len(body) < length:
        chunk = stream.read(length - len(body))
        if not chunk:
            break
        body += chunk
    return bytes(prefix) + bytes(body)


def frame_prefix_length(frame):
    for index, byte in enumerate(frame):
        if byte & 0x80 == 0:
            return index + 1
    return len(frame)


def read_sftp_packet(stream):
    length_bytes = stream.read(4)
    if not length_bytes:
        return None
    if len(length_bytes) != 4:
        return length_bytes
    length = int.from_bytes(length_bytes, "big")
    body = bytearray()
    while len(body) < length:
        chunk = stream.read(length - len(body))
        if not chunk:
            break
        body += chunk
    return length_bytes + bytes(body)


def sftp_packet_type(packet):
    return packet[4] if len(packet) >= 5 else None


def sentinel():
    path = os.environ.get("RUXEL_CHAOS_SENTINEL")
    if path:
        Path(path).write_text("observed\n")


def cut_and_exit(child, output, frame, require_large=False):
    prefix = frame_prefix_length(frame)
    body = len(frame) - prefix
    if require_large and body < LARGE_FRAME_BYTES:
        child.kill()
        os._exit(87)
    keep = prefix + max(1, body // 2)
    output.write(frame[:keep])
    output.flush()
    sentinel()
    child.kill()
    os._exit(86)


def copy_stderr(child):
    while chunk := child.stderr.read(65536):
        sys.stderr.buffer.write(chunk)
        sys.stderr.buffer.flush()


def proxy_frames(child, fault):
    def controller_to_agent():
        count = 0
        while (frame := read_frame(sys.stdin.buffer)) is not None:
            count += 1
            if fault == "large-plan" and len(frame) >= LARGE_FRAME_BYTES:
                cut_and_exit(child, child.stdin, frame, require_large=True)
            child.stdin.write(frame)
            child.stdin.flush()
        child.stdin.close()

    def agent_to_controller():
        count = 0
        while (frame := read_frame(child.stdout)) is not None:
            count += 1
            if fault == "partial-hello-ack" and count == 1:
                cut_and_exit(child, sys.stdout.buffer, frame)
            if fault == "large-task-result" and len(frame) >= LARGE_FRAME_BYTES:
                cut_and_exit(child, sys.stdout.buffer, frame, require_large=True)
            sys.stdout.buffer.write(frame)
            sys.stdout.buffer.flush()
            if (fault == "long-subprocess" and count == 4) or (
                fault == "controlmaster-sigint" and count == 1
            ):
                sentinel()

    threads = [
        threading.Thread(target=controller_to_agent, daemon=True),
        threading.Thread(target=agent_to_controller, daemon=True),
        threading.Thread(target=copy_stderr, args=(child,), daemon=True),
    ]
    for thread in threads:
        thread.start()
    return child.wait()


def proxy_sftp(child):
    def upload():
        while (packet := read_sftp_packet(sys.stdin.buffer)) is not None:
            if sftp_packet_type(packet) == 6:  # SSH_FXP_WRITE
                child.stdin.write(packet[: max(5, len(packet) // 2)])
                child.stdin.flush()
                sentinel()
                time.sleep(120)
                return
            child.stdin.write(packet)
            child.stdin.flush()

    threading.Thread(target=upload, daemon=True).start()
    threading.Thread(target=copy_stderr, args=(child,), daemon=True).start()
    while chunk := child.stdout.read(65536):
        sys.stdout.buffer.write(chunk)
        sys.stdout.buffer.flush()
    return child.wait()


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    real_ssh = os.environ.get("RUXEL_CHAOS_REAL_SSH", "/usr/bin/ssh")
    fault = os.environ.get("RUXEL_CHAOS_CASE", "")
    if not fault or (not is_agent_command(argv) and not is_sftp_command(argv)):
        os.execv(real_ssh, [real_ssh, *argv])

    child = subprocess.Popen(
        [real_ssh, *argv],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=0,
    )
    if fault == "upload-start" and is_sftp_command(argv):
        return proxy_sftp(child)
    if is_agent_command(argv):
        return proxy_frames(child, fault)
    return child.wait()


if __name__ == "__main__":
    raise SystemExit(main())
