#!/usr/bin/env python3
"""Build the private, local-only AEC Stage 0 helper.

This intentionally uses a separate target directory and never copies an AEC
binary into Tauri's externalBin directory. It is not a release build path.
"""

from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
TAURI_ROOT = ROOT / "app" / "src-tauri"
TARGET_DIR = TAURI_ROOT / "target" / "aec-spike"


def main() -> int:
    if sys.platform != "darwin":
        raise SystemExit("AEC feasibility tooling requires macOS")
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(TARGET_DIR)
    env["MURMUR_AEC_SPIKE_BUILD_SHA"] = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip()
    subprocess.run(
        [
            "cargo",
            "build",
            "-p",
            "murmur-capture-helper",
            "--features",
            "aec-spike",
        ],
        cwd=TAURI_ROOT,
        env=env,
        check=True,
    )
    binary = TARGET_DIR / "debug" / "murmur-capture-helper"
    if not binary.is_file():
        raise SystemExit(f"AEC helper build did not produce {binary}")
    print(binary)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
