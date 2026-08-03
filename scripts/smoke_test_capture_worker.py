#!/usr/bin/env python3
"""Exercise the final signed capture worker's production-v3 startup protocol."""

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
PROTOCOL_VERSION = 3
HEADER_BYTES = 36
MAX_CONTROL_BYTES = 16 * 1024


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
        [str(worker), "--production-v3", str(capture_id), nonce.hex()],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
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


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=5.0)
    arguments = parser.parse_args()
    try:
        smoke_test(arguments.worker, arguments.timeout_seconds)
    except SmokeError as error:
        raise SystemExit(f"ERROR: {error}") from error
    print("signed capture worker production-v3 startup smoke passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
