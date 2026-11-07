#!/usr/bin/env python3
"""Fail closed if any target/feature/dev/build dependency of the core is a UI host."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    result = subprocess.run(
        [
            "cargo", "tree", "--locked", "-p", "murmur-core",
            "--target", "all", "--all-features", "--edges", "all",
            "--prefix", "none", "--format", "{p}",
        ],
        cwd=ROOT / "app",
        text=True,
        stdout=subprocess.PIPE,
        check=True,
    )
    packages = {line.split()[0] for line in result.stdout.splitlines() if line.strip()}
    if "murmur-core" not in packages:
        raise RuntimeError("cargo tree did not include murmur-core")
    forbidden = sorted(name for name in packages if name in {"tauri", "wry", "tao"} or name.startswith("tauri-")
                       or name == "gpui" or name.startswith(("gpui-", "gpui_")))
    if forbidden:
        print("Core depends on forbidden host packages: " + ", ".join(forbidden), file=sys.stderr)
        return 1
    print("murmur-core: no Tauri or GPUI dependencies (all targets, features, dev/build edges)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
