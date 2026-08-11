#!/usr/bin/env python3
"""Exercise the capture worker's production-v3 startup protocol."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import secrets
import selectors
import signal
import struct
import subprocess
import time
from typing import BinaryIO


MAGIC = b"MRMR"
PROTOCOL_VERSION = 3
HEADER_BYTES = 36
MAX_CONTROL_BYTES = 16 * 1024
MAX_PCM_SAMPLES = 16 * 1024
MIN_PCM_FRAMES = 3

SETUP_STEPS = {
    "auhal": (
        "deviceResolution",
        "audioUnitNew",
        "enableInputIo",
        "disableOutputIo",
        "setCurrentDevice",
        "formatConfiguration",
        "callbackInstallation",
        "streamStart",
    ),
    "cpal": (
        "deviceResolution",
        "defaultConfig",
        "streamBuild",
        "streamStart",
    ),
}


class SmokeError(RuntimeError):
    pass


@dataclass(frozen=True)
class PcmFrame:
    sequence: int
    sample_rate: int
    sample_count: int


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
            if remaining <= 0 or not selector.select(remaining):
                raise SmokeError("capture worker response timed out")
            chunk = os.read(stream.fileno(), length - len(chunks))
            if not chunk:
                raise SmokeError("capture worker closed its protocol pipe")
            chunks.extend(chunk)
    finally:
        selector.close()
    return bytes(chunks)


def _discard_exact(stream: BinaryIO, length: int, deadline: float) -> None:
    remaining = length
    while remaining:
        chunk = _read_exact(stream, min(remaining, 4096), deadline)
        remaining -= len(chunk)


def read_production_frame(
    stream: BinaryIO, capture_id: int, nonce: bytes, deadline: float
) -> dict[str, object] | PcmFrame:
    header = _read_exact(stream, HEADER_BYTES, deadline)
    magic, version, kind, reserved, length, actual_capture_id, actual_nonce = (
        struct.unpack("<4sHBBIQ16s", header)
    )
    if (
        magic != MAGIC
        or version != PROTOCOL_VERSION
        or reserved != 0
        or actual_capture_id != capture_id
        or actual_nonce != nonce
    ):
        raise SmokeError("capture worker returned an invalid protocol header")

    if kind == 0:
        if length > MAX_CONTROL_BYTES:
            raise SmokeError("capture worker returned an oversized control frame")
        payload = _read_exact(stream, length, deadline)
        try:
            message = json.loads(payload)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise SmokeError("capture worker returned invalid control JSON") from error
        if not isinstance(message, dict):
            raise SmokeError("capture worker returned a non-object control message")
        return message

    if kind != 1 or not 16 <= length <= 16 + MAX_PCM_SAMPLES * 4:
        raise SmokeError("capture worker returned an invalid PCM frame header")
    metadata = _read_exact(stream, 16, deadline)
    sequence, sample_rate, sample_count = struct.unpack("<QII", metadata)
    if (
        sample_count == 0
        or sample_count > MAX_PCM_SAMPLES
        or length != 16 + sample_count * 4
    ):
        raise SmokeError("capture worker returned a malformed PCM frame")
    # Consume but never decode, print, persist, or upload PCM content.
    _discard_exact(stream, sample_count * 4, deadline)
    return PcmFrame(sequence, sample_rate, sample_count)


def read_control_frame(
    stream: BinaryIO, capture_id: int, nonce: bytes, deadline: float
) -> dict[str, object]:
    frame = read_production_frame(stream, capture_id, nonce, deadline)
    if not isinstance(frame, dict):
        raise SmokeError("capture worker returned PCM where control was required")
    return frame


def terminate_worker(process: subprocess.Popen[bytes]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        if process.poll() is not None:
            return
        process.terminate()
    except PermissionError:
        process.terminate()
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            if process.poll() is None:
                process.kill()
        except PermissionError:
            process.kill()
        process.wait(timeout=2)


def _write_control(
    process: subprocess.Popen[bytes],
    capture_id: int,
    nonce: bytes,
    message: dict[str, object],
) -> None:
    if process.stdin is None:
        raise SmokeError("capture worker protocol input is unavailable")
    process.stdin.write(encode_control_frame(capture_id, nonce, message))
    process.stdin.flush()


def _expect_control(
    stream: BinaryIO,
    capture_id: int,
    nonce: bytes,
    deadline: float,
    expected: dict[str, object],
) -> None:
    actual = read_control_frame(stream, capture_id, nonce, deadline)
    if actual.get("type") == "failure":
        raise SmokeError(f"capture worker failed: {actual}")
    if actual != expected:
        raise SmokeError(f"expected control message {expected}, received {actual}")


def _start_worker(worker: Path) -> tuple[subprocess.Popen[bytes], int, bytes]:
    capture_id = secrets.randbits(63) or 1
    nonce = os.urandom(16)
    process = subprocess.Popen(
        [str(worker), "--production-v3", str(capture_id), nonce.hex()],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    return process, capture_id, nonce


def smoke_test(worker: Path, timeout_seconds: float = 5.0) -> None:
    """Keep the hardware-free stream-open smoke used by release builders."""
    if not worker.is_file() or not os.access(worker, os.X_OK):
        raise SmokeError("capture worker is missing or not executable")
    process, capture_id, nonce = _start_worker(worker)
    try:
        if process.stdout is None:
            raise SmokeError("capture worker protocol output is unavailable")
        deadline = time.monotonic() + timeout_seconds
        _write_control(process, capture_id, nonce, {"type": "hello"})
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            deadline,
            {"type": "helloAck"},
        )
        _write_control(
            process,
            capture_id,
            nonce,
            {"type": "start", "deviceId": None, "backend": "auhal"},
        )
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            deadline,
            {"type": "phase", "phase": "streamOpen", "backend": "auhal"},
        )
    finally:
        terminate_worker(process)
        for stream in (process.stdin, process.stdout, process.stderr):
            if stream is not None:
                stream.close()


def _smoke_backend_to_first_pcm(
    worker: Path,
    device_id: str,
    backend: str,
    timeout_seconds: float,
    first_pcm_seconds: float,
) -> float:
    process, capture_id, nonce = _start_worker(worker)
    try:
        if process.stdout is None:
            raise SmokeError("capture worker protocol output is unavailable")
        hard_deadline = time.monotonic() + timeout_seconds
        _write_control(process, capture_id, nonce, {"type": "hello"})
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            hard_deadline,
            {"type": "helloAck"},
        )

        started_at = time.monotonic()
        first_pcm_deadline = min(hard_deadline, started_at + first_pcm_seconds)
        _write_control(
            process,
            capture_id,
            nonce,
            {"type": "start", "deviceId": device_id, "backend": backend},
        )
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            first_pcm_deadline,
            {"type": "phase", "phase": "streamOpen", "backend": backend},
        )
        for step in SETUP_STEPS[backend]:
            for transition in ("entered", "completed"):
                _expect_control(
                    process.stdout,
                    capture_id,
                    nonce,
                    first_pcm_deadline,
                    {
                        "type": "setupStep",
                        "backend": backend,
                        "step": step,
                        "transition": transition,
                    },
                )
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            first_pcm_deadline,
            {
                "type": "setupStep",
                "backend": backend,
                "step": "awaitingFirstCallback",
                "transition": "entered",
            },
        )
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            first_pcm_deadline,
            {
                "type": "phase",
                "phase": "awaitingFirstCallback",
                "backend": backend,
            },
        )
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            first_pcm_deadline,
            {
                "type": "setupStep",
                "backend": backend,
                "step": "awaitingFirstCallback",
                "transition": "completed",
            },
        )
        _expect_control(
            process.stdout,
            capture_id,
            nonce,
            first_pcm_deadline,
            {"type": "phase", "phase": "active", "backend": backend},
        )

        pcm_frames = 0
        pcm_samples = 0
        expected_sequence = 0
        first_pcm_elapsed = 0.0
        while pcm_frames < MIN_PCM_FRAMES:
            deadline = first_pcm_deadline if pcm_frames == 0 else hard_deadline
            frame = read_production_frame(
                process.stdout, capture_id, nonce, deadline
            )
            if isinstance(frame, dict):
                if frame.get("type") == "failure":
                    raise SmokeError(f"capture worker failed: {frame}")
                raise SmokeError(f"expected PCM frame, received {frame}")
            if frame.sequence != expected_sequence:
                raise SmokeError(
                    f"expected PCM sequence {expected_sequence}, received {frame.sequence}"
                )
            if frame.sample_rate <= 0:
                raise SmokeError("capture worker returned an invalid PCM sample rate")
            if pcm_frames == 0:
                first_pcm_elapsed = time.monotonic() - started_at
                if first_pcm_elapsed >= first_pcm_seconds:
                    raise SmokeError(
                        f"{backend} first PCM took {first_pcm_elapsed:.3f}s "
                        f"(limit {first_pcm_seconds:.3f}s)"
                    )
            pcm_frames += 1
            pcm_samples += frame.sample_count
            expected_sequence += 1

        _write_control(process, capture_id, nonce, {"type": "stop"})
        while True:
            frame = read_production_frame(
                process.stdout, capture_id, nonce, hard_deadline
            )
            if isinstance(frame, PcmFrame):
                if frame.sequence != expected_sequence:
                    raise SmokeError(
                        f"expected PCM sequence {expected_sequence}, received {frame.sequence}"
                    )
                expected_sequence += 1
                pcm_samples += frame.sample_count
                continue
            if frame.get("type") == "failure":
                raise SmokeError(f"capture worker failed while stopping: {frame}")
            if frame.get("type") != "stopped":
                raise SmokeError(f"expected stopped response, received {frame}")
            retained = frame.get("retainedSamples")
            if not isinstance(retained, int) or retained < pcm_samples:
                raise SmokeError("capture worker reported an invalid retained sample count")
            break

        remaining = hard_deadline - time.monotonic()
        if remaining <= 0:
            raise SmokeError("capture worker hard timeout expired before clean exit")
        try:
            return_code = process.wait(timeout=remaining)
        except subprocess.TimeoutExpired as error:
            raise SmokeError("capture worker did not exit after stopped") from error
        if return_code != 0:
            raise SmokeError(f"capture worker exited with status {return_code}")
        return first_pcm_elapsed
    finally:
        terminate_worker(process)
        for stream in (process.stdin, process.stdout, process.stderr):
            if stream is not None:
                stream.close()


def smoke_test_to_first_pcm(
    worker: Path,
    device_id: str,
    timeout_seconds: float = 8.0,
    first_pcm_seconds: float = 2.0,
) -> dict[str, float]:
    if not worker.is_file() or not os.access(worker, os.X_OK):
        raise SmokeError("capture worker is missing or not executable")
    if not device_id.strip():
        raise SmokeError("an explicit capture device UID is required")
    if timeout_seconds <= 0 or first_pcm_seconds <= 0:
        raise SmokeError("timeouts must be positive")
    return {
        backend: _smoke_backend_to_first_pcm(
            worker,
            device_id,
            backend,
            timeout_seconds,
            first_pcm_seconds,
        )
        for backend in ("auhal", "cpal")
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--worker", required=True, type=Path)
    parser.add_argument(
        "--device-id",
        help="run the deep AUHAL+CPAL smoke against this explicit Core Audio UID",
    )
    parser.add_argument("--timeout-seconds", type=float, default=8.0)
    parser.add_argument("--first-pcm-seconds", type=float, default=2.0)
    arguments = parser.parse_args()
    try:
        if arguments.device_id:
            timings = smoke_test_to_first_pcm(
                arguments.worker,
                arguments.device_id,
                arguments.timeout_seconds,
                arguments.first_pcm_seconds,
            )
        else:
            smoke_test(arguments.worker, arguments.timeout_seconds)
            timings = None
    except SmokeError as error:
        raise SystemExit(f"ERROR: {error}") from error
    if timings is None:
        print("signed capture worker production-v3 startup smoke passed")
    else:
        summary = ", ".join(
            f"{backend} first_pcm={elapsed:.3f}s"
            for backend, elapsed in timings.items()
        )
        print(f"capture worker first-PCM smoke passed: {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
