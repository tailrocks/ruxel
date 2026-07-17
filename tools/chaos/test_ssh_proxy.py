import io
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import ssh_proxy as proxy
from make_payload import SIZE, write_payload


def framed(body):
    length = len(body)
    prefix = bytearray()
    while length >= 128:
        prefix.append((length & 0x7F) | 0x80)
        length >>= 7
    prefix.append(length)
    return bytes(prefix) + body


class SshProxyTests(unittest.TestCase):
    def test_reads_one_and_multi_byte_frame_lengths(self):
        for size in (3, 300):
            expected = framed(b"x" * size)
            self.assertEqual(proxy.read_frame(io.BytesIO(expected)), expected)

    def test_clean_eof_and_truncated_frame_are_bounded(self):
        self.assertIsNone(proxy.read_frame(io.BytesIO()))
        self.assertEqual(proxy.read_frame(io.BytesIO(b"\x05ab")), b"\x05ab")

    def test_large_boundary_is_at_least_one_mebibyte(self):
        self.assertEqual(proxy.LARGE_FRAME_BYTES, 1024 * 1024)

    def test_generated_plan_payload_crosses_large_boundary(self):
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "payload"
            write_payload(path)
            self.assertEqual(path.stat().st_size, SIZE)
            self.assertGreater(SIZE, proxy.LARGE_FRAME_BYTES)

    def test_classifies_only_agent_exec_and_sftp_channels(self):
        self.assertTrue(proxy.is_agent_command(["host", "--", "/var/lib/ruxel/agent/hash"]))
        self.assertFalse(proxy.is_agent_command(["host", "--", "/bin/true"]))
        self.assertTrue(proxy.is_sftp_command(["host", "-s", "sftp"]))
        self.assertFalse(proxy.is_sftp_command(["host", "--", "sftp-helper"]))

    def test_sftp_write_boundary_uses_packet_type(self):
        body = bytes([6]) + b"write-body"
        packet = len(body).to_bytes(4, "big") + body
        self.assertEqual(proxy.read_sftp_packet(io.BytesIO(packet)), packet)
        self.assertEqual(proxy.sftp_packet_type(packet), 6)


if __name__ == "__main__":
    unittest.main()
