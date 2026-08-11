from __future__ import annotations

import os
from pathlib import Path
import stat
import tempfile
import textwrap
import time
import unittest

from scripts.smoke_test_capture_worker import (
    encode_control_frame,
    PcmFrame,
    read_control_frame,
    read_production_frame,
    SmokeError,
    smoke_test,
    smoke_test_to_first_pcm,
)


FAKE_DEEP_WORKER = textwrap.dedent(
    """\
    #!/usr/bin/env python3
    import json
    import struct
    import sys

    capture_id = int(sys.argv[2])
    nonce = bytes.fromhex(sys.argv[3])

    def read_frame():
        header = sys.stdin.buffer.read(36)
        magic, version, kind, reserved, length, actual_id, actual_nonce = struct.unpack("<4sHBBIQ16s", header)
        assert (magic, version, kind, reserved, actual_id, actual_nonce) == (b"MRMR", 3, 0, 0, capture_id, nonce)
        return json.loads(sys.stdin.buffer.read(length))

    def write_control(message):
        body = json.dumps(message, separators=(",", ":")).encode()
        sys.stdout.buffer.write(struct.pack("<4sHBBIQ16s", b"MRMR", 3, 0, 0, len(body), capture_id, nonce) + body)
        sys.stdout.buffer.flush()

    def write_pcm(sequence):
        body = struct.pack("<QIIf", sequence, 48000, 1, 0.25)
        sys.stdout.buffer.write(struct.pack("<4sHBBIQ16s", b"MRMR", 3, 1, 0, len(body), capture_id, nonce) + body)
        sys.stdout.buffer.flush()

    assert read_frame() == {"type": "hello"}
    write_control({"type": "helloAck"})
    start = read_frame()
    backend = start["backend"]
    assert start == {"type": "start", "deviceId": "fixture-device-uid", "backend": backend}
    steps = {
        "auhal": ["deviceResolution", "audioUnitNew", "enableInputIo", "disableOutputIo", "setCurrentDevice", "formatConfiguration", "callbackInstallation", "streamStart"],
        "cpal": ["deviceResolution", "defaultConfig", "streamBuild", "streamStart"],
    }[backend]
    write_control({"type": "phase", "phase": "streamOpen", "backend": backend})
    for step in steps:
        for transition in ("entered", "completed"):
            write_control({"type": "setupStep", "backend": backend, "step": step, "transition": transition})
    write_control({"type": "setupStep", "backend": backend, "step": "awaitingFirstCallback", "transition": "entered"})
    write_control({"type": "phase", "phase": "awaitingFirstCallback", "backend": backend})
    write_control({"type": "setupStep", "backend": backend, "step": "awaitingFirstCallback", "transition": "completed"})
    write_control({"type": "phase", "phase": "active", "backend": backend})
    for sequence in range(3):
        write_pcm(sequence)
    assert read_frame() == {"type": "stop"}
    write_control({"type": "stopped", "retainedSamples": 3})
    """
)


class CaptureWorkerSmokeTests(unittest.TestCase):
    def _worker(self, directory: str, contents: str) -> Path:
        worker = Path(directory) / "fake-worker"
        worker.write_text(contents, encoding="utf-8")
        worker.chmod(worker.stat().st_mode | stat.S_IXUSR)
        return worker

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

    def test_pcm_frame_is_counted_without_exposing_content(self) -> None:
        capture_id = 42
        nonce = bytes(range(16))
        body = b"".join(
            (
                (7).to_bytes(8, "little"),
                (48_000).to_bytes(4, "little"),
                (2).to_bytes(4, "little"),
                b"private!",
            )
        )
        header = (
            b"MRMR"
            + (3).to_bytes(2, "little")
            + bytes((1, 0))
            + len(body).to_bytes(4, "little")
            + capture_id.to_bytes(8, "little")
            + nonce
        )
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, header + body)
        finally:
            os.close(write_fd)
        with os.fdopen(read_fd, "rb", buffering=0) as stream:
            self.assertEqual(
                read_production_frame(
                    stream, capture_id, nonce, time.monotonic() + 1
                ),
                PcmFrame(sequence=7, sample_rate=48_000, sample_count=2),
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

    def test_protocol_only_smoke_exercises_hello_and_start(self) -> None:
        fake_worker = textwrap.dedent(
            """\
            #!/usr/bin/env python3
            import json
            import struct
            import sys

            capture_id = int(sys.argv[2])
            nonce = bytes.fromhex(sys.argv[3])

            def read_frame():
                header = sys.stdin.buffer.read(36)
                magic, version, kind, reserved, length, actual_id, actual_nonce = struct.unpack("<4sHBBIQ16s", header)
                assert (magic, version, kind, reserved, actual_id, actual_nonce) == (b"MRMR", 3, 0, 0, capture_id, nonce)
                return json.loads(sys.stdin.buffer.read(length))

            def write_frame(message):
                body = json.dumps(message, separators=(",", ":")).encode()
                sys.stdout.buffer.write(struct.pack("<4sHBBIQ16s", b"MRMR", 3, 0, 0, len(body), capture_id, nonce) + body)
                sys.stdout.buffer.flush()

            assert read_frame() == {"type": "hello"}
            write_frame({"type": "helloAck"})
            assert read_frame() == {"type": "start", "deviceId": None, "backend": "auhal"}
            write_frame({"type": "phase", "phase": "streamOpen", "backend": "auhal"})
            sys.stdin.buffer.read()
            """
        )
        with tempfile.TemporaryDirectory() as directory:
            smoke_test(self._worker(directory, fake_worker), timeout_seconds=2)

    def test_deep_smoke_exercises_both_backends_to_three_pcm_frames(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            timings = smoke_test_to_first_pcm(
                self._worker(directory, FAKE_DEEP_WORKER),
                "fixture-device-uid",
                timeout_seconds=2,
                first_pcm_seconds=1,
            )
        self.assertEqual(set(timings), {"auhal", "cpal"})
        self.assertTrue(all(0 <= timing < 1 for timing in timings.values()))

    def test_deep_smoke_rejects_out_of_order_setup_steps(self) -> None:
        out_of_order = FAKE_DEEP_WORKER.replace(
            '["deviceResolution", "defaultConfig", "streamBuild", "streamStart"]',
            '["defaultConfig", "deviceResolution", "streamBuild", "streamStart"]',
        )
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(SmokeError, "expected control message"):
                smoke_test_to_first_pcm(
                    self._worker(directory, out_of_order),
                    "fixture-device-uid",
                    timeout_seconds=2,
                    first_pcm_seconds=1,
                )

    def test_deep_smoke_hard_times_out_and_kills_worker(self) -> None:
        hanging = FAKE_DEEP_WORKER.replace(
            'write_control({"type": "phase", "phase": "streamOpen", "backend": backend})',
            'import time; time.sleep(60)',
        )
        with tempfile.TemporaryDirectory() as directory:
            started = time.monotonic()
            with self.assertRaisesRegex(SmokeError, "timed out"):
                smoke_test_to_first_pcm(
                    self._worker(directory, hanging),
                    "fixture-device-uid",
                    timeout_seconds=0.2,
                    first_pcm_seconds=0.1,
                )
            self.assertLess(time.monotonic() - started, 3)


if __name__ == "__main__":
    unittest.main()
