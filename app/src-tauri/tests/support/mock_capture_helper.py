#!/usr/bin/python3
"""Deterministic content-free fault helper for issue #407 integration tests."""

from __future__ import annotations

import json
import os
import struct
import subprocess
import sys
import time


PROTOCOL = "murmur.capture_probe"
VERSION = 1


def block_forever() -> None:
    while True:
        time.sleep(60)


def read_frame() -> dict:
    header = sys.stdin.buffer.read(4)
    if len(header) != 4:
        raise SystemExit(70)
    length = struct.unpack(">I", header)[0]
    if length > 4096:
        raise SystemExit(70)
    body = sys.stdin.buffer.read(length)
    if len(body) != length:
        raise SystemExit(70)
    return json.loads(body)


def write_frame(frame: dict) -> None:
    body = json.dumps(frame, separators=(",", ":")).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()


def write_raw(body: bytes, declared_length: int | None = None) -> None:
    length = len(body) if declared_length is None else declared_length
    sys.stdout.buffer.write(struct.pack(">I", length) + body)
    sys.stdout.buffer.flush()


def base(frame_type: str, nonce: str, **fields: object) -> dict:
    return {
        "type": frame_type,
        "protocol": PROTOCOL,
        "version": VERSION,
        "sessionNonce": nonce,
        **fields,
    }


def main() -> None:
    scenario = os.environ.get("MOCK_CAPTURE_SCENARIO", "happy")
    if scenario == "hang_then_happy":
        marker = os.environ["MOCK_CAPTURE_MARKER"]
        if os.path.exists(marker):
            scenario = "happy"
        else:
            with open(marker, "x", encoding="utf-8"):
                pass
            scenario = "pre_handshake_block"
    if scenario == "pre_handshake_block":
        block_forever()

    hello = read_frame()
    if hello.get("type") != "hello":
        raise SystemExit(70)
    nonce = str(hello["sessionNonce"])
    if scenario == "wrong_nonce":
        write_frame(base("phase", nonce + "-wrong", phase="enumeration"))
        block_forever()
    if scenario == "wrong_version":
        frame = base("phase", nonce, phase="enumeration")
        frame["version"] = VERSION + 1
        write_frame(frame)
        block_forever()
    if scenario == "malformed":
        write_raw(b"{not-json")
        block_forever()
    if scenario == "truncated":
        write_raw(b"{", declared_length=100)
        os.close(sys.stdout.fileno())
        block_forever()
    if scenario == "oversized":
        write_raw(b"", declared_length=4097)
        block_forever()
    if scenario == "ready_out_of_order":
        write_frame(base("ready", nonce))
        block_forever()
    write_frame(base("phase", nonce, phase="enumeration"))
    if scenario == "duplicate_phase":
        write_frame(base("phase", nonce, phase="enumeration"))
        block_forever()
    if scenario == "enumeration_block":
        block_forever()
    write_frame(base("phase", nonce, phase="streamOpen"))
    if scenario == "open_block":
        block_forever()
    write_frame(base("ready", nonce))
    if scenario == "ready_without_awaiting":
        block_forever()
    if scenario == "first_callback_without_awaiting":
        write_frame(base("firstCallback", nonce, callbackLatencyMs=1))
        block_forever()
    write_frame(base("phase", nonce, phase="awaitingFirstCallback"))
    if scenario != "starts_without_callbacks":
        write_frame(base("firstCallback", nonce, callbackLatencyMs=1))
        if scenario == "missing_active":
            block_forever()
        if scenario == "delayed_active":
            time.sleep(0.25)
        write_frame(base("phase", nonce, phase="active"))
    if scenario == "phase_regression":
        write_frame(base("phase", nonce, phase="enumeration"))
        block_forever()
    if scenario in {"after_first_audio_block", "starts_without_callbacks"}:
        block_forever()
    if scenario == "descendant_block":
        subprocess.Popen(["/bin/sleep", "60"])
        block_forever()

    while True:
        message = read_frame()
        if message.get("type") != "cancel" or message.get("sessionNonce") != nonce:
            raise SystemExit(70)
        if scenario == "ignore_cancel":
            block_forever()
        write_frame(base("phase", nonce, phase="stopping"))
        if scenario == "graceful_stop_block":
            block_forever()
        write_frame(base("stopped", nonce))
        return


if __name__ == "__main__":
    main()
