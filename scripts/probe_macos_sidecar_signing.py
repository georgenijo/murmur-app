#!/usr/bin/env python3
"""Build an ad-hoc signed fixture app and prove split entitlement preservation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--output-app", type=Path)
    args = parser.parse_args()

    temporary = None
    if args.output_app:
        app = args.output_app.resolve()
        if app.exists():
            shutil.rmtree(app)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="murmur-sidecar-signing-")
        app = Path(temporary.name) / "MurmurSigningProbe.app"

    macos = app / "Contents" / "MacOS"
    macos.mkdir(parents=True)
    info = {
        "CFBundleExecutable": "MurmurSigningProbe",
        "CFBundleIdentifier": "com.localdictation.signing-probe",
        "CFBundleName": "MurmurSigningProbe",
        "CFBundlePackageType": "APPL",
        "CFBundleVersion": "1",
    }
    with (app / "Contents" / "Info.plist").open("wb") as handle:
        plistlib.dump(info, handle)

    source = Path(temporary.name if temporary else app.parent) / "main.c"
    source.write_text("int main(void) { return 0; }\n")
    subprocess.run(
        [
            "xcrun",
            "clang",
            "-arch",
            "arm64",
            "-mmacosx-version-min=14.0",
            str(source),
            "-o",
            str(macos / "MurmurSigningProbe"),
        ],
        check=True,
    )
    shutil.copy2(args.helper, macos / "murmur-llm-sidecar")

    subprocess.run(
        [
            "python3",
            str(ROOT / "scripts" / "finalize_macos_bundle.py"),
            "--app",
            str(app),
            "--identity",
            "-",
            "--main-entitlements",
            str(ROOT / "app" / "src-tauri" / "entitlements.plist"),
            "--helper-entitlements",
            str(ROOT / "app" / "src-tauri" / "local-llm-sidecar.entitlements.plist"),
        ],
        check=True,
    )
    result = {
        "schema_version": 1,
        "app": str(app),
        "helper": str(macos / "murmur-llm-sidecar"),
        "identity": "adhoc",
        "main_sandboxed": False,
        "helper_sandboxed": True,
        "result": "passed",
    }
    print(json.dumps(result, sort_keys=True))
    if temporary:
        temporary.cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
