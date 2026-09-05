#!/usr/bin/env python3
"""Exercise the final signed capture worker's production-v9 startup protocol."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import secrets
import selectors
import struct
import subprocess
import time
from typing import BinaryIO


MAGIC = b"MRMR"
PROTOCOL_VERSION = 9
HEADER_BYTES = 36
MAX_CONTROL_BYTES = 16 * 1024
MAX_PCM_SAMPLES = 16 * 1024


class SmokeError(RuntimeError):
    pass


def encode_control_frame(
    capture_id: int, nonce: bytes, message: dict[str, object]
) -> bytes:
    if len(nonce) != 16:
        raise SmokeError("session nonce must contain exactly 16 bytes")
    payload = json.dumps(message, separators=(",", ":")).encode("utf-8")
    if len(payload) > MAX_CONTROL_BYTES:
        raise SmokeError("control payload exceeds the protocol limit")
    header = struct.pack(
        "<4sHBBIQ16s",
        MAGIC,
        PROTOCOL_VERSION,
        0,
        0,
        len(payload),
        capture_id,
        nonce,
    )
    return header + payload


def _read_exact(stream: BinaryIO, length: int, deadline: float) -> bytes:
    chunks = bytearray()
    selector = selectors.DefaultSelector()
    selector.register(stream, selectors.EVENT_READ)
    try:
        while len(chunks) < length:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise SmokeError("capture worker response timed out")
            if not selector.select(remaining):
                raise SmokeError("capture worker response timed out")
            chunk = os.read(stream.fileno(), length - len(chunks))
            if not chunk:
                raise SmokeError("capture worker closed its protocol pipe")
            chunks.extend(chunk)
    finally:
        selector.close()
    return bytes(chunks)


def read_control_frame(
    stream: BinaryIO, capture_id: int, nonce: bytes, deadline: float
) -> dict[str, object]:
    header = _read_exact(stream, HEADER_BYTES, deadline)
    magic, version, kind, reserved, length, actual_capture_id, actual_nonce = (
        struct.unpack("<4sHBBIQ16s", header)
    )
    if (
        magic != MAGIC
        or version != PROTOCOL_VERSION
        or kind != 0
        or reserved != 0
        or actual_capture_id != capture_id
        or actual_nonce != nonce
        or length > MAX_CONTROL_BYTES
    ):
        raise SmokeError("capture worker returned an invalid protocol header")
    payload = _read_exact(stream, length, deadline)
    try:
        message = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SmokeError("capture worker returned invalid control JSON") from error
    if not isinstance(message, dict):
        raise SmokeError("capture worker returned a non-object control message")
    return message


def read_production_frame(
    stream: BinaryIO, capture_id: int, nonce: bytes, deadline: float
) -> tuple[str, dict[str, object]]:
    header = _read_exact(stream, HEADER_BYTES, deadline)
    magic, version, kind, channel, length, actual_capture_id, actual_nonce = (
        struct.unpack("<4sHBBIQ16s", header)
    )
    if (
        magic != MAGIC
        or version != PROTOCOL_VERSION
        or actual_capture_id != capture_id
        or actual_nonce != nonce
    ):
        raise SmokeError("capture worker returned an invalid protocol header")
    is_control = kind == 0 and channel == 0 and length <= MAX_CONTROL_BYTES
    is_pcm = (
        kind == 1
        and channel in (1, 2)
        and 32 <= length <= 32 + MAX_PCM_SAMPLES * 4
    )
    if not is_control and not is_pcm:
        raise SmokeError("capture worker returned an unsupported production frame")
    payload = _read_exact(stream, length, deadline)
    if is_control:
        try:
            message = json.loads(payload)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise SmokeError("capture worker returned invalid control JSON") from error
        if not isinstance(message, dict):
            raise SmokeError("capture worker returned a non-object control message")
        return "control", message
    if is_pcm:
        sample_count = struct.unpack_from("<I", payload, 12)[0]
        if sample_count == 0 or length != 32 + sample_count * 4:
            raise SmokeError("capture worker returned malformed PCM")
        return "pcm", {
            "channel": "microphone" if channel == 1 else "system",
            "sampleCount": sample_count,
        }
    raise AssertionError("validated production frame kind was not handled")


def terminate_worker(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=2)


def smoke_test(worker: Path, timeout_seconds: float = 5.0) -> None:
    if not worker.is_file() or not os.access(worker, os.X_OK):
        raise SmokeError("capture worker is missing or not executable")
    capture_id = secrets.randbits(63) or 1
    nonce = os.urandom(16)
    process = subprocess.Popen(
        [str(worker), "--production-v9", str(capture_id), nonce.hex()],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        if process.stdin is None or process.stdout is None:
            raise SmokeError("capture worker protocol pipes are unavailable")
        deadline = time.monotonic() + timeout_seconds
        process.stdin.write(
            encode_control_frame(capture_id, nonce, {"type": "hello"})
        )
        process.stdin.flush()
        hello = read_control_frame(process.stdout, capture_id, nonce, deadline)
        if hello != {"type": "helloAck"}:
            raise SmokeError("capture worker did not acknowledge protocol hello")

        process.stdin.write(
            encode_control_frame(
                capture_id,
                nonce,
                {"type": "start", "deviceId": None, "backend": "auhal"},
            )
        )
        process.stdin.flush()
        phase = read_control_frame(process.stdout, capture_id, nonce, deadline)
        if phase != {
            "type": "phase",
            "phase": "streamOpen",
            "backend": "auhal",
        }:
            raise SmokeError("capture worker did not enter the stream-open phase")
    finally:
        terminate_worker(process)
        for stream in (process.stdin, process.stdout, process.stderr):
            if stream is not None:
                stream.close()


def smoke_test_meeting(worker: Path, timeout_seconds: float = 20.0) -> None:
    if not worker.is_file() or not os.access(worker, os.X_OK):
        raise SmokeError("capture worker is missing or not executable")
    capture_id = secrets.randbits(63) or 1
    nonce = os.urandom(16)
    process = subprocess.Popen(
        [str(worker), "--production-v9", str(capture_id), nonce.hex()],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    last_setup = "none"
    active_channels: set[str] = set()
    try:
        if process.stdin is None or process.stdout is None:
            raise SmokeError("capture worker protocol pipes are unavailable")
        deadline = time.monotonic() + timeout_seconds
        process.stdin.write(
            encode_control_frame(capture_id, nonce, {"type": "hello"})
        )
        process.stdin.flush()
        hello = read_control_frame(process.stdout, capture_id, nonce, deadline)
        if hello != {"type": "helloAck"}:
            raise SmokeError("capture worker did not acknowledge protocol hello")

        process.stdin.write(
            encode_control_frame(
                capture_id,
                nonce,
                {
                    "type": "startMeeting",
                    "deviceId": None,
                    "backend": "auhal",
                    "echoCancellation": "disabled",
                },
            )
        )
        process.stdin.flush()
        while active_channels != {"microphone", "system"}:
            frame_kind, frame = read_production_frame(
                process.stdout, capture_id, nonce, deadline
            )
            if frame_kind == "control" and frame.get("type") == "meetingSetupStep":
                last_setup = (
                    f"{frame.get('channel')}:{frame.get('step')}:{frame.get('transition')}"
                )
            elif frame_kind == "control" and frame.get("type") == "meetingPhase":
                if frame.get("phase") == "active" and isinstance(frame.get("channel"), str):
                    active_channels.add(str(frame["channel"]))
            elif frame_kind == "control" and frame.get("type") == "meetingFailure":
                raise SmokeError(
                    f"meeting capture failed at {last_setup}: {frame.get('code')}"
                )

        process.stdin.write(
            encode_control_frame(capture_id, nonce, {"type": "stop"})
        )
        process.stdin.flush()
        while True:
            frame_kind, frame = read_production_frame(
                process.stdout, capture_id, nonce, deadline
            )
            if frame_kind == "control" and frame.get("type") == "meetingStopped":
                break
            if frame_kind == "control" and frame.get("type") == "meetingFailure":
                raise SmokeError(
                    f"meeting capture failed while stopping at {last_setup}: {frame.get('code')}"
                )
    except SmokeError as error:
        raise SmokeError(f"{error}; last setup transition={last_setup}") from error
    finally:
        terminate_worker(process)
        for stream in (process.stdin, process.stdout, process.stderr):
            if stream is not None:
                stream.close()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=5.0)
    parser.add_argument("--meeting", action="store_true")
    arguments = parser.parse_args()
    try:
        if arguments.meeting:
            smoke_test_meeting(arguments.worker, max(arguments.timeout_seconds, 20.0))
        else:
            smoke_test(arguments.worker, arguments.timeout_seconds)
    except SmokeError as error:
        raise SystemExit(f"ERROR: {error}") from error
    if arguments.meeting:
        print("signed capture worker production-v9 meeting smoke passed")
    else:
        print("signed capture worker production-v9 startup smoke passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
