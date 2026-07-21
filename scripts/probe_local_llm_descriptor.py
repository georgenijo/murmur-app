#!/usr/bin/env python3
"""Prove a signed sandboxed sidecar can load a verified inherited GGUF fd."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import stat
import struct
import subprocess
import time


PROTOCOL = "murmur.local_llm"
VERSION = 1
MODEL_FD = 3
MAX_FRAME = 64 * 1024


def frame(payload: dict[str, object]) -> bytes:
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    if len(body) > MAX_FRAME:
        raise ValueError("probe message exceeds protocol frame limit")
    return struct.pack(">I", len(body)) + body


def read_exact(stream, length: int, deadline: float) -> bytes:
    result = bytearray()
    while len(result) < length:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("sidecar response timed out")
        ready, _, _ = select.select([stream], [], [], remaining)
        if not ready:
            raise TimeoutError("sidecar response timed out")
        chunk = os.read(stream.fileno(), length - len(result))
        if not chunk:
            raise EOFError("sidecar closed stdout")
        result.extend(chunk)
    return bytes(result)


def read_frame(stream, timeout_seconds: float) -> dict[str, object]:
    deadline = time.monotonic() + timeout_seconds
    length = struct.unpack(">I", read_exact(stream, 4, deadline))[0]
    if length > MAX_FRAME:
        raise ValueError("sidecar response exceeds frame limit")
    return json.loads(read_exact(stream, length, deadline))


def hash_fd(fd: int) -> tuple[int, str]:
    metadata = os.fstat(fd)
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError("model descriptor is not a regular file")
    digest = hashlib.sha256()
    while chunk := os.read(fd, 1024 * 1024):
        digest.update(chunk)
    os.lseek(fd, 0, os.SEEK_SET)
    return metadata.st_size, digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--model-id", default="qwen2.5-1.5b-instruct-q4_k_m")
    parser.add_argument("--expected-size", type=int, required=True)
    parser.add_argument("--expected-sha256", required=True)
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    helper = args.helper.resolve()
    model = args.model.resolve()

    model_fd = os.open(model, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        if model_fd != MODEL_FD:
            os.dup2(model_fd, MODEL_FD, inheritable=True)
            os.close(model_fd)
            model_fd = MODEL_FD
        os.set_inheritable(model_fd, True)
        size, sha256 = hash_fd(model_fd)
        if size != args.expected_size or sha256 != args.expected_sha256:
            raise SystemExit("model size or SHA-256 does not match the signed catalog identity")

        process = subprocess.Popen(
            [str(helper)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            close_fds=True,
            pass_fds=(model_fd,),
            env={},
            cwd="/",
        )
        assert process.stdin is not None and process.stdout is not None
        nonce = "stage0-descriptor-probe"
        hello = {
            "type": "hello",
            "protocol": PROTOCOL,
            "version": VERSION,
            "sessionNonce": nonce,
            "model": {"id": args.model_id, "sha256": sha256, "sizeBytes": size},
            "limits": {
                "maxFrameBytes": 65536,
                "maxInstructionBytes": 4096,
                "maxInputBytes": 16384,
                "maxOutputBytes": 16384,
                "maxOutputTokens": 2048,
                "maxContextTokens": 8192,
                "maxDeadlineMs": 30000,
            },
        }
        process.stdin.write(frame(hello))
        process.stdin.flush()
        try:
            ready = read_frame(process.stdout, 45.0)
        except (EOFError, TimeoutError) as error:
            process.wait(timeout=5.0)
            assert process.stderr is not None
            debug_stderr = process.stderr.read().decode("utf-8", errors="replace").strip()
            detail = debug_stderr or f"exit={process.returncode} with release stderr suppressed"
            raise SystemExit(f"sidecar handshake failed closed: {detail}") from error
        if ready.get("type") != "ready" or ready.get("sessionNonce") != nonce:
            raise SystemExit(f"sidecar did not complete the descriptor handshake: {ready}")
        backend = str(ready.get("backend", ""))
        if not backend.lower().startswith("metal:"):
            raise SystemExit(f"sidecar did not prove Metal inference backend availability: {backend}")

        request_id = "stage0-fixed-transform"
        process.stdin.write(
            frame(
                {
                    "type": "transform",
                    "protocol": PROTOCOL,
                    "version": VERSION,
                    "sessionNonce": nonce,
                    "requestId": request_id,
                    "instruction": "Return the input text unchanged.",
                    "input": "Murmur stage zero.",
                    "maxOutputTokens": 32,
                    "deadlineMs": 30000,
                }
            )
        )
        process.stdin.flush()
        transformed = read_frame(process.stdout, 35.0)
        if (
            transformed.get("type") != "result"
            or transformed.get("requestId") != request_id
            or not str(transformed.get("output", "")).strip()
        ):
            raise SystemExit(f"sidecar did not complete fixed Metal inference: {transformed}")

        process.stdin.write(
            frame(
                {
                    "type": "shutdown",
                    "protocol": PROTOCOL,
                    "version": VERSION,
                    "sessionNonce": nonce,
                }
            )
        )
        process.stdin.flush()
        stopped = read_frame(process.stdout, 5.0)
        if stopped.get("type") != "stopped":
            raise SystemExit(f"sidecar did not stop cleanly: {stopped}")
        process.wait(timeout=5.0)
        if process.returncode != 0:
            raise SystemExit(f"sidecar exited with {process.returncode}")

        evidence = {
            "schema_version": 1,
            "sandboxed_helper": str(helper),
            "model_id": args.model_id,
            "model_size": size,
            "model_sha256": sha256,
            "backend": backend,
            "fixed_inference": "passed",
            "protocol_version": VERSION,
            "result": "passed",
        }
        if args.evidence:
            args.evidence.parent.mkdir(parents=True, exist_ok=True)
            args.evidence.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
        print(json.dumps(evidence, sort_keys=True))
    finally:
        os.close(model_fd)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
