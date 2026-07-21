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
import tempfile
import time


PROTOCOL = "murmur.local_llm"
VERSION = 1
MODEL_FD = 3
MAX_FRAME = 64 * 1024

# Signed-catalog identity of the only v1 model (see the signed-local-LLM ADR).
# Used by --skip-model to drive the helper's descriptor check without shipping
# the 1.1 GB GGUF into CI.
CATALOG_SIZE = 1_117_320_736
CATALOG_SHA256 = "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e"

# The helper's main() maps every controlled run() error (protocol mismatch, limit
# mismatch, and every descriptor rejection in verify_inherited_model) to this
# fixed exit code and suppresses stderr in release builds. Dyld/arch/sandbox/
# signature launch failures instead surface as an OSError at spawn time or a
# signal death (negative return code) or a shell/loader code such as 126/127 —
# never a clean 70 — so exit==70 uniquely identifies a controlled fail-closed
# rejection rather than a crash at launch. See sidecars/local-llm/src/main.rs.
HELPER_CONTROLLED_EXIT = 70


def _limits() -> dict[str, int]:
    return {
        "maxFrameBytes": 65536,
        "maxInstructionBytes": 4096,
        "maxInputBytes": 16384,
        "maxOutputBytes": 16384,
        "maxOutputTokens": 2048,
        "maxContextTokens": 8192,
        "maxDeadlineMs": 30000,
    }


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


def run_skip_model(helper: Path, model_id: str, evidence: Path | None) -> int:
    """Prove spawn + failure-closed descriptor handling without the real model.

    A bogus, tiny fd 3 is inherited while the hello frame advertises the signed
    catalog identity. This exercises the signed sandboxed binary's ability to
    spawn and its fail-closed model verification while deferring the full Metal/
    inference proof (which needs the cached model).

    Fail-closed is only accepted when the helper rejects the descriptor through a
    *controlled* path, distinguished from a crash at launch (dyld/arch/sandbox/
    signature failure):

      * an explicit protocol ``error`` frame as the first frame, or
      * a clean exit with the helper's dedicated controlled-error code and no
        ``ready``/``result`` frame.

    A ``ready``/``result`` frame, a zero exit, a signal death, or any other exit
    code is treated as a probe FAILURE. The temp fd is managed without a live
    Python file object across ``dup2`` so fd 3 is never double-closed.
    """
    fd, bogus_path = tempfile.mkstemp(prefix="murmur-bogus-model-")
    try:
        os.write(fd, b"not the real gguf model")
        os.close(fd)  # release the writable fd before opening the read-only fd 3

        model_fd = os.open(bogus_path, os.O_RDONLY | os.O_NOFOLLOW)
        try:
            if model_fd != MODEL_FD:
                os.dup2(model_fd, MODEL_FD, inheritable=True)
                os.close(model_fd)
                model_fd = MODEL_FD
            os.set_inheritable(model_fd, True)

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
            nonce = "skip-model-failure-closed-probe"
            hello = {
                "type": "hello",
                "protocol": PROTOCOL,
                "version": VERSION,
                "sessionNonce": nonce,
                "model": {
                    "id": model_id,
                    "sha256": CATALOG_SHA256,
                    "sizeBytes": CATALOG_SIZE,
                },
                "limits": _limits(),
            }
            try:
                process.stdin.write(frame(hello))
                process.stdin.flush()
            except BrokenPipeError as error:
                process.wait(timeout=5.0)
                raise SystemExit(
                    "sidecar closed stdin before reading hello; likely crashed at "
                    f"launch (exit={process.returncode!r})"
                ) from error

            first: dict[str, object] | None
            try:
                first = read_frame(process.stdout, 30.0)
            except (EOFError, TimeoutError):
                first = None
            frame_type = first.get("type") if isinstance(first, dict) else None
            if frame_type in ("ready", "result"):
                raise SystemExit(
                    f"sidecar produced a {frame_type!r} frame for a mismatched "
                    "descriptor; fail-closed is broken"
                )

            try:
                process.wait(timeout=15.0)
            except subprocess.TimeoutExpired:
                process.kill()
                raise SystemExit("sidecar did not exit after descriptor rejection")

            stderr_text = ""
            if process.stderr is not None:
                stderr_text = (
                    process.stderr.read().decode("utf-8", errors="replace").strip()
                )

            error_code = None
            if frame_type == "error":
                # Future-proof path: an explicit protocol error frame is itself a
                # fail-closed signal. Record its code. (The current helper rejects
                # descriptors by exiting rather than framing, so this branch is
                # typically not taken.)
                error_code = first.get("code") if isinstance(first, dict) else None
                discriminator = "error-frame"
            elif frame_type is None and process.returncode == HELPER_CONTROLLED_EXIT:
                discriminator = "controlled-exit-code"
            else:
                detail = stderr_text or "release stderr suppressed"
                raise SystemExit(
                    "sidecar did not fail closed through a controlled path: "
                    f"exit={process.returncode!r} first_frame={first!r} ({detail})"
                )

            evidence_payload = {
                "schema_version": 1,
                "sandboxed_helper": str(helper),
                "mode": "skip-model",
                "model_id": model_id,
                "expected_size": CATALOG_SIZE,
                "expected_sha256": CATALOG_SHA256,
                "emitted_ready": False,
                "discriminator": discriminator,
                "helper_exit_code": process.returncode,
                "helper_error_code": error_code,
                "helper_stderr": stderr_text,  # empty in release (stderr suppressed)
                "helper_first_frame": first,
                "fixed_inference": "deferred (needs cached model)",
                "protocol_version": VERSION,
                "result": "failed-closed-as-expected",
            }
            if evidence:
                evidence.parent.mkdir(parents=True, exist_ok=True)
                evidence.write_text(
                    json.dumps(evidence_payload, indent=2, sort_keys=True) + "\n"
                )
            print(json.dumps(evidence_payload, sort_keys=True))
        finally:
            os.close(model_fd)
    finally:
        try:
            os.unlink(bogus_path)
        except OSError:
            pass
    return 0


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--model", type=Path)
    parser.add_argument("--model-id", default="qwen2.5-1.5b-instruct-q4_k_m")
    parser.add_argument("--expected-size", type=int)
    parser.add_argument("--expected-sha256")
    parser.add_argument(
        "--skip-model",
        action="store_true",
        help="prove spawn + fail-closed descriptor rejection without the real model",
    )
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    helper = args.helper.resolve()

    if args.skip_model:
        return run_skip_model(helper, args.model_id, args.evidence)

    if args.model is None or args.expected_size is None or args.expected_sha256 is None:
        raise SystemExit(
            "--model, --expected-size, and --expected-sha256 are required "
            "unless --skip-model is set"
        )
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
