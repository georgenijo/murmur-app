#!/usr/bin/env python3
"""Assemble the native bundle without Tauri. Never signs or publishes.

The native bundle manifest owns identity and version. The frozen pre-flip
identity contract checks permission descriptions, helpers and entitlements.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import plistlib
import shutil

from finalize_macos_bundle import HELPERS, install_capture_agent_plist, require_exact_macos_executables

ROOT = Path(__file__).resolve().parents[1]
BUNDLE_ROOT = ROOT / "app/macos"


def bundle_info(bundle_root: Path = BUNDLE_ROOT) -> dict:
    config = json.loads((bundle_root / "bundle.json").read_text())
    mac = config["bundle"]["macOS"]
    name = config["productName"]
    info = {
        "CFBundleDevelopmentRegion": "English",
        "CFBundleDisplayName": name,
        "CFBundleExecutable": "ui",
        "CFBundleIdentifier": config["identifier"],
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundleName": name,
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": config["version"],
        "CFBundleVersion": config["version"],
        "CFBundleIconFile": "icon.icns",
        "CSResourcesFileMapped": True,
        "LSMinimumSystemVersion": mac["minimumSystemVersion"],
        "LSRequiresCarbon": True,
        "NSHighResolutionCapable": True,
    }
    # Preserve even the empty exception-domain dictionary emitted by Tauri.
    if mac.get("exceptionDomain") is not None:
        info["NSAppTransportSecurity"] = {"NSExceptionDomains": {
            mac["exceptionDomain"]: {
                "NSExceptionAllowsInsecureHTTPLoads": True,
                "NSIncludesSubdomains": True,
            }
        }}
    with (bundle_root / mac["infoPlist"]).open("rb") as handle:
        info.update(plistlib.load(handle))
    if info["CFBundleExecutable"] != "ui" or info["LSMinimumSystemVersion"] != "14.0":
        raise ValueError("native bundle must use ui and macOS 14.0")
    return info


def bundle(binary: Path, output: Path, bundle_root: Path = BUNDLE_ROOT, *, helpers_dir: Path | None = None) -> Path:
    info = bundle_info(bundle_root)
    if output.suffix != ".app" or output.exists():
        raise ValueError("output must be a new .app directory")
    sources = {"ui": binary, **{
        name: (helpers_dir if helpers_dir is not None else ROOT / "app/binaries") / f"{name}-aarch64-apple-darwin" for name in HELPERS
    }}
    for source in sources.values():
        if not source.is_file():
            raise ValueError(f"missing binary: {source}")
    macos = output / "Contents/MacOS"
    resources = output / "Contents/Resources"
    macos.mkdir(parents=True)
    resources.mkdir()
    with (output / "Contents/Info.plist").open("wb") as handle:
        plistlib.dump(info, handle)
    shutil.copy2(bundle_root / "icons/icon.icns", resources / "icon.icns")
    for name, source in sources.items():
        shutil.copy2(source, macos / name)
        (macos / name).chmod(0o755)
    install_capture_agent_plist(output, bundle_root / "macos/com.localdictation.capture-agent.plist")
    require_exact_macos_executables(output, macos / "ui", [macos / name for name in HELPERS])
    return output


def compare_bundles(native: Path, tauri: Path) -> dict:
    """No ignored plist keys or layout paths except the codesign resource seal.

    Both bundles must have been finalized (installing the launchd plist). Bundle
    binaries differ by design; the resource and launch-agent bytes must match.
    """
    def info(app):
        with (app / "Contents/Info.plist").open("rb") as handle:
            return plistlib.load(handle)

    a, b = info(native), info(tauri)
    if a != b:
        keys = sorted(k for k in a.keys() | b.keys() if a.get(k) != b.get(k) or (k in a) != (k in b))
        raise ValueError(f"Info.plist differs: {keys}")

    def layout(app):
        return {p.relative_to(app).as_posix() + ("/" if p.is_dir() else "")
                for p in app.rglob("*") if "_CodeSignature" not in p.parts}

    left, right = layout(native), layout(tauri)
    if left != right:
        raise ValueError(f"bundle layout differs: native-only={sorted(left-right)}, tauri-only={sorted(right-left)}")
    for relative in sorted(left):
        if relative.startswith(("Contents/Resources/", "Contents/Library/")) and not relative.endswith("/"):
            if (native / relative).read_bytes() != (tauri / relative).read_bytes():
                raise ValueError(f"bundle resource differs: {relative}")
    for app in (native, tauri):
        macos = app / "Contents/MacOS"
        require_exact_macos_executables(app, macos / "ui", [macos / name for name in HELPERS])
    return {"plist_keys": sorted(a), "plist_diff": [], "layout": sorted(left), "layout_diff": []}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(bundle(args.binary.resolve(), args.output.resolve()))


if __name__ == "__main__":
    main()
