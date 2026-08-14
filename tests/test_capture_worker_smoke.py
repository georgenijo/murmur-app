from __future__ import annotations

import os
from pathlib import Path
import stat
import struct
import tempfile
import textwrap
import time
import unittest

from scripts.smoke_test_capture_worker import (
    encode_control_frame,
    read_control_frame,
    SmokeError,
    smoke_test,
)


class CaptureWorkerSmokeTests(unittest.TestCase):
    def test_control_frame_round_trip(self) -> None:
        capture_id = 42
        nonce = bytes(range(16))
        encoded = encode_control_frame(capture_id, nonce, {"type": "helloAck"})
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, encoded)
        finally:
            os.close(write_fd)
        with os.fdopen(read_fd, "rb", buffering=0) as stream:
            self.assertEqual(
                read_control_frame(
                    stream, capture_id, nonce, time.monotonic() + 1
                ),
                {"type": "helloAck"},
            )

    def test_invalid_capture_identity_fails_closed(self) -> None:
        nonce = bytes(range(16))
        encoded = encode_control_frame(41, nonce, {"type": "helloAck"})
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, encoded)
        finally:
            os.close(write_fd)
        with os.fdopen(read_fd, "rb", buffering=0) as stream:
            with self.assertRaisesRegex(SmokeError, "invalid protocol header"):
                read_control_frame(stream, 42, nonce, time.monotonic() + 1)

    def test_v5_protocol_version_fails_closed(self) -> None:
        capture_id = 42
        nonce = bytes(range(16))
        encoded = bytearray(
            encode_control_frame(capture_id, nonce, {"type": "helloAck"})
        )
        encoded[4:6] = struct.pack("<H", 5)
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, encoded)
        finally:
            os.close(write_fd)
        with os.fdopen(read_fd, "rb", buffering=0) as stream:
            with self.assertRaisesRegex(SmokeError, "invalid protocol header"):
                read_control_frame(
                    stream, capture_id, nonce, time.monotonic() + 1
                )

    def test_smoke_exercises_hello_and_start(self) -> None:
        fake_worker = textwrap.dedent(
            """\
            #!/usr/bin/env python3
            import json
            import os
            import struct
            import sys

            assert sys.argv[1] == "--production-v6"
            capture_id = int(sys.argv[2])
            nonce = bytes.fromhex(sys.argv[3])

            def read_frame():
                header = sys.stdin.buffer.read(36)
                magic, version, kind, reserved, length, actual_id, actual_nonce = struct.unpack("<4sHBBIQ16s", header)
                assert (magic, version, kind, reserved, actual_id, actual_nonce) == (b"MRMR", 6, 0, 0, capture_id, nonce)
                return json.loads(sys.stdin.buffer.read(length))

            def write_frame(message):
                body = json.dumps(message, separators=(",", ":")).encode()
                sys.stdout.buffer.write(struct.pack("<4sHBBIQ16s", b"MRMR", 6, 0, 0, len(body), capture_id, nonce) + body)
                sys.stdout.buffer.flush()

            assert read_frame() == {"type": "hello"}
            write_frame({"type": "helloAck"})
            assert read_frame() == {"type": "start", "deviceId": None, "backend": "auhal"}
            write_frame({"type": "phase", "phase": "streamOpen", "backend": "auhal"})
            sys.stdin.buffer.read()
            """
        )
        with tempfile.TemporaryDirectory() as directory:
            worker = Path(directory) / "fake-worker"
            worker.write_text(fake_worker, encoding="utf-8")
            worker.chmod(worker.stat().st_mode | stat.S_IXUSR)
            smoke_test(worker, timeout_seconds=2)


if __name__ == "__main__":
    unittest.main()
