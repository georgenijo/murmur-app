#!/usr/bin/env python3
"""Bounded native AUHAL/CPAL smoke. Only counts and timings reach stdout."""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import json
import math
import os
from pathlib import Path
import secrets
import selectors
import signal
import subprocess
import sys
import time

try:
    from .smoke_test_capture_worker import (
        SmokeError, encode_control_frame, read_production_frame,
    )
except ImportError:
    from smoke_test_capture_worker import (
        SmokeError, encode_control_frame, read_production_frame,
    )

STEPS = {
    "auhal": ["deviceResolution", "audioUnitNew", "enableInputIo", "disableOutputIo",
              "setCurrentDevice", "formatConfiguration", "callbackInstallation",
              "streamStart", "awaitingFirstCallback"],
    "cpal": ["deviceResolution", "defaultConfig", "streamBuild", "streamStart",
             "awaitingFirstCallback"],
}
CLEANUP_SECONDS = 1.0


def live_group_members(group_id: int, timeout: float) -> bool:
    try:
        result = subprocess.run(
            ["/bin/ps", "-axo", "pgid=,stat="], capture_output=True, check=True,
            timeout=timeout, text=True,
        )
    except subprocess.TimeoutExpired as error:
        raise SmokeError("owned worker cleanup exceeded its deadline") from error
    for row in result.stdout.splitlines():
        fields = row.split()
        if len(fields) == 2 and fields[0] == str(group_id) and not fields[1].startswith("Z"):
            return True
    return False


def terminate_group(process: subprocess.Popen[bytes]) -> None:
    # Keep the leader unreaped until signaling, so its PID cannot be reused.
    deadline = time.monotonic() + CLEANUP_SECONDS
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        # Darwin reports EPERM for an exited, unreaped group leader. The bounded
        # reap and group check below must still prove no running member remains.
        pass
    try:
        process.wait(timeout=CLEANUP_SECONDS)
    except subprocess.TimeoutExpired as error:
        raise SmokeError("owned worker did not exit after hard kill") from error
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise SmokeError("owned worker cleanup exceeded its deadline")
    if live_group_members(process.pid, remaining):
        raise SmokeError("owned worker group still has running members")


def wait_for_exit(process: subprocess.Popen[bytes], deadline: float) -> None:
    # Apple's system Python does not expose waitid/WNOWAIT. Observe the zombie
    # without reaping it, then read its exit status after owned-group cleanup.
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SmokeError("worker did not exit after stop acknowledgment")
        try:
            result = subprocess.run(
                ["/bin/ps", "-o", "stat=", "-p", str(process.pid)],
                capture_output=True, text=True, timeout=remaining,
            )
        except subprocess.TimeoutExpired as error:
            raise SmokeError("worker did not exit after stop acknowledgment") from error
        if result.returncode != 0:
            raise SmokeError("worker exit state could not be observed")
        if result.stdout.strip().startswith("Z"):
            if time.monotonic() >= deadline:
                raise SmokeError("worker did not exit after stop acknowledgment")
            return
        time.sleep(min(max(0, deadline - time.monotonic()), 0.005))


class Session:
    def __init__(self, process: subprocess.Popen[bytes], deadline: float,
                 capture_id: int, nonce: bytes):
        self.process = process
        self.deadline = deadline
        self.capture_id = capture_id
        self.nonce = nonce
        self.require_clean_exit = False

    def send(self, message: dict[str, object]) -> None:
        data = memoryview(encode_control_frame(self.capture_id, self.nonce, message))
        with selectors.DefaultSelector() as selector:
            selector.register(self.process.stdin, selectors.EVENT_WRITE)
            while data:
                remaining = self.deadline - time.monotonic()
                if remaining <= 0 or not selector.select(remaining):
                    raise SmokeError("worker command timed out")
                try:
                    count = os.write(self.process.stdin.fileno(), data)
                except BlockingIOError:
                    continue
                data = data[count:]

    def read(self, deadline: float | None = None):
        return read_production_frame(
            self.process.stdout, self.capture_id, self.nonce,
            min(self.deadline, deadline or self.deadline),
        )


@contextmanager
def worker_session(worker: Path, timeout: float):
    if not worker.is_file() or not os.access(worker, os.X_OK):
        raise SmokeError("worker is missing or not executable")
    capture_id = secrets.randbits(63) or 1
    nonce = secrets.token_bytes(16)
    deadline = time.monotonic() + timeout
    process = subprocess.Popen(
        [str(worker), "--production-v10", str(capture_id), nonce.hex()],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        start_new_session=True, bufsize=0,
    )
    session = None
    try:
        session = Session(process, deadline, capture_id, nonce)
        os.set_blocking(process.stdin.fileno(), False)
        session.send({"type": "hello"})
        if session.read() != ("control", {"type": "helloAck"}):
            raise SmokeError("worker did not acknowledge protocol v10")
        yield session
    finally:
        try:
            terminate_group(process)
            if session is not None and session.require_clean_exit and process.returncode != 0:
                raise SmokeError("worker exited unsuccessfully after stopping")
        finally:
            process.stdin.close()
            process.stdout.close()


def select_builtin_input(worker: Path, timeout: float = 5.0) -> str:
    with worker_session(worker, timeout) as session:
        session.send({"type": "enumerate"})
        kind, frame = session.read()
        if kind != "control" or frame.get("type") != "devices":
            raise SmokeError("worker did not return input inventory")
        devices = frame.get("devices")
        if not isinstance(devices, list) or not all(isinstance(d, dict) for d in devices):
            raise SmokeError("worker returned malformed inventory")
        eligible = [d for d in devices if d.get("kind") == "builtIn"
                    and d.get("connected") is True and d.get("hasInput") is True]
        default = [d for d in eligible if d.get("id") == frame.get("defaultInputId")]
        selected = default if default else eligible
        if frame.get("lidState") != "open" or len(selected) != 1:
            raise SmokeError("runner needs an open lid and an unambiguous connected built-in input")
        uid = selected[0].get("id")
        if not isinstance(uid, str) or not uid or len(uid.encode()) > 4096:
            raise SmokeError("built-in input selector is invalid")
        return uid


def run_backend(worker: Path, backend: str, device_id: str, *,
                timeout: float = 10.0, first_pcm_limit: float = 2.0,
                stop_limit: float = 2.0) -> dict[str, object]:
    expected_steps = [(step, transition) for step in STEPS[backend]
                      for transition in ("entered", "completed")]
    expected_phases = ["streamOpen", "awaitingFirstCallback", "active"]
    phase_step_counts = [0, len(expected_steps) - 1, len(expected_steps)]
    step_count = phase_count = frame_count = sample_count = 0
    resolved_input = False
    sample_rate = None
    last_timestamp = -1
    first_pcm_ms = None
    stop_at = None
    with worker_session(worker, timeout) as session:
        started = time.monotonic()
        session.send({"type": "start", "deviceId": device_id, "backend": backend})
        while True:
            deadline = (stop_at + stop_limit if stop_at is not None else
                        started + first_pcm_limit if first_pcm_ms is None else session.deadline)
            kind, frame = session.read(deadline)
            now = time.monotonic()
            if kind == "pcm":
                if not resolved_input or step_count != len(expected_steps) or phase_count != len(expected_phases):
                    raise SmokeError("PCM arrived before complete setup and active phase")
                if (frame["channel"] != "microphone" or frame["sequence"] != frame_count
                        or frame["sampleOffset"] != sample_count
                        or frame["capturedAtNs"] < last_timestamp
                        or (sample_rate is not None and frame["sampleRate"] != sample_rate)):
                    raise SmokeError("microphone PCM metadata is inconsistent")
                sample_rate = frame["sampleRate"]
                last_timestamp = frame["capturedAtNs"]
                frame_count += 1
                sample_count += frame["sampleCount"]
                if first_pcm_ms is None:
                    first_pcm_ms = (now - started) * 1000
                    if now - started >= first_pcm_limit:
                        raise SmokeError("first PCM exceeded startup bound")
                if frame_count >= 3 and stop_at is None:
                    stop_at = time.monotonic()
                    session.send({"type": "stop"})
                continue
            message_type = frame.get("type")
            if message_type == "stopped":
                if stop_at is None or frame.get("retainedSamples") != sample_count:
                    raise SmokeError("stop acknowledgment has inconsistent sample count")
                if now >= stop_at + stop_limit:
                    raise SmokeError("stop acknowledgment exceeded bound")
                wait_for_exit(session.process, min(session.deadline, stop_at + stop_limit))
                session.require_clean_exit = True
                return {"backend": backend, "pcm_frames": frame_count,
                        "samples": sample_count, "setup_steps": len(STEPS[backend]),
                        "first_pcm_ms": round(first_pcm_ms, 2),
                        "stop_ms": round((now - stop_at) * 1000, 2)}
            if frame.get("backend") != backend:
                raise SmokeError("worker changed backend or returned an unexpected message")
            if message_type == "setupStep":
                observed = (frame.get("step"), frame.get("transition"))
                if step_count >= len(expected_steps) or observed != expected_steps[step_count]:
                    raise SmokeError("native setup steps are unbalanced or out of order")
                required_phase_count = 2 if step_count == len(expected_steps) - 1 else 1
                if phase_count != required_phase_count:
                    raise SmokeError("native setup step arrived outside its capture phase")
                step_count += 1
            elif message_type == "phase":
                if phase_count >= len(expected_phases) or frame.get("phase") != expected_phases[phase_count]:
                    raise SmokeError("capture phases are out of order")
                if step_count != phase_step_counts[phase_count]:
                    raise SmokeError("capture phase arrived outside its native setup step")
                phase_count += 1
            elif message_type == "inputResolution":
                if (resolved_input or step_count != 1
                        or frame.get("inputEnumerationOk") is not True
                        or frame.get("requestedPresent") is not True):
                    raise SmokeError("pinned input was not resolved")
                resolved_input = True
            else:
                # Never echo worker-controlled strings, IDs, names, or raw errors.
                raise SmokeError("worker reported failure or an unexpected control message")


def positive_seconds(value: str) -> float:
    result = float(value)
    if not math.isfinite(result) or result <= 0 or result > 30:
        raise argparse.ArgumentTypeError("deadline must be finite and in (0, 30] seconds")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=positive_seconds, default=10.0)
    parser.add_argument("--first-pcm-seconds", type=positive_seconds, default=2.0)
    parser.add_argument("--inventory-only", action="store_true")
    args = parser.parse_args()
    def interrupted(_signum, _frame):
        raise SmokeError("capture smoke interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    backend = "inventory"
    try:
        device_id = select_builtin_input(args.worker, args.timeout_seconds)
        if args.inventory_only:
            print(json.dumps({"selected_builtin_inputs": 1, "lid_open": True}))
            return 0
        reports = []
        for backend in STEPS:
            reports.append(run_backend(args.worker, backend, device_id,
                                       timeout=args.timeout_seconds,
                                       first_pcm_limit=args.first_pcm_seconds))
    except (SmokeError, OSError, subprocess.SubprocessError) as error:
        # OSError can include filesystem names. Keep external errors out of logs.
        print(json.dumps({"passed": False, "backend": backend, "error": str(error) if isinstance(error, SmokeError)
                          else "worker process or pipe operation failed"}), file=sys.stderr)
        return 1
    print(json.dumps({"passed": True, "protocol": 10, "backends": reports}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
