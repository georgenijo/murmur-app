#!/usr/bin/env python3
"""Secretless native packaging proof. Run on an isolated arm64 macOS checkout.

Uses Tauri's *bundler* as an independent packaging oracle, with the native ui
binary as payload in both bundles. It does not build or exercise the Tauri host.
No credentials, persistent profiles, launchd registration or publication.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import sys
import time
import tempfile

from bundle_gpui import ROOT, TAURI, bundle, bundle_info, compare_bundles
from finalize_macos_bundle import HELPERS, require_exact_macos_executables


def run(command: list[str], **kwargs) -> str:
    return subprocess.check_output(command, text=True, **kwargs)


def finalize(app: Path, identity: str, team: str | None = None) -> None:
    command = [sys.executable, str(ROOT / "scripts/finalize_macos_bundle.py"),
               "--app", str(app), "--identity", identity]
    for flag, source in {
        "main-entitlements": "entitlements.plist",
        "capture-agent-entitlements": "capture-agent.entitlements.plist",
        "capture-helper-entitlements": "capture-helper.entitlements.plist",
        "capture-worker-entitlements": "capture-worker.entitlements.plist",
        "capture-agent-info-plist": "capture-agent-info.plist",
        "capture-helper-info-plist": "sidecars/capture/Info.plist",
        "capture-worker-info-plist": "sidecars/capture/WorkerInfo.plist",
        "capture-agent-launchd-plist": "macos/com.localdictation.capture-agent.plist",
    }.items():
        command += ["--" + flag, str(TAURI / source)]
    if team:
        command += ["--expected-team-id", team]
    subprocess.run(command, check=True)


def check_links(binary: Path) -> dict:
    loads = run(["otool", "-l", str(binary)])
    if "path /usr/lib/swift " not in loads:
        raise ValueError("Swift runtime rpath missing")
    sdk = Path(run(["xcrun", "--sdk", "macosx", "--show-sdk-path"]).strip())
    framework = sdk / "System/Library/Frameworks/FoundationModels.framework"
    commands = loads.split("Load command ")
    fm = [c for c in commands if "FoundationModels.framework/" in c]
    if framework.exists() and (len(fm) != 1 or "cmd LC_LOAD_WEAK_DYLIB" not in fm[0]):
        raise ValueError("FoundationModels must be weak-linked")
    build = [c for c in commands if "cmd LC_BUILD_VERSION" in c]
    if not build or not any("minos 14.0" in c for c in build):
        raise ValueError("Mach-O minimum must be macOS 14.0")
    return {"swift_rpath": "/usr/lib/swift", "macos_minimum": "14.0",
            "foundation_models": "weak" if fm else "SDK unavailable"}


def isolated_launch(native: Path, output: Path, env: dict[str, str]) -> str:
    # The parity bundle keeps production metadata but must never run the full
    # service host against the invoking user's profile. Launch a disposable copy
    # with an unknown/non-production identifier and a synthetic HOME instead.
    with tempfile.TemporaryDirectory(prefix="murmur-gpui-test-packaging-") as temp:
        test_root = Path(temp)
        test_app = test_root / "Murmur Packaging QA.app"
        shutil.copytree(native, test_app)
        plist = test_app / "Contents/Info.plist"
        info = plistlib.loads(plist.read_bytes())
        info["CFBundleIdentifier"] = "com.localdictation.gpui-packaging-qa"
        plist.write_bytes(plistlib.dumps(info))
        finalize(test_app, "-")
        home = test_root / "home"
        home.mkdir()
        launch_env = {**env, "HOME": str(home), "CFFIXED_USER_HOME": str(home)}
        with (output / "launch.log").open("w") as log:
            child = subprocess.Popen([str(test_app / "Contents/MacOS/ui")],
                                     stdout=log, stderr=log, env=launch_env)
            try:
                time.sleep(3)
                if child.poll() is not None:
                    raise ValueError("packaged production entry exited during isolated launch check")
                result = "production entry; isolated identity/HOME; exact test PID terminated"
            finally:
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="new rehearsal directory")
    parser.add_argument("--skip-build", action="store_true", help="use this checkout's already-built native ui")
    args = parser.parse_args()
    if sys.platform != "darwin" or os.uname().machine != "arm64":
        parser.error("requires arm64 macOS")
    # Inspect names only, never load credential values. Even empty secret slots
    # are forbidden so this lane cannot accidentally grow a signing dependency.
    forbidden = {"APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID", "APPLE_CERTIFICATE",
                 "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY", "TAURI_SIGNING_PRIVATE_KEY",
                 "TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "TAURI_PRIVATE_KEY", "TAURI_KEY_PASSWORD"}
    if forbidden.intersection(os.environ):
        parser.error("release credential environment is forbidden in the secretless rehearsal")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    target = args.target_dir.resolve()
    env = {**os.environ, "CARGO_TARGET_DIR": str(target), "RUSTUP_TOOLCHAIN": "1.98.1",
           "MACOSX_DEPLOYMENT_TARGET": "14.0", "CMAKE_OSX_DEPLOYMENT_TARGET": "14.0"}
    metadata = json.loads(run(["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT / "app/gpui", env=env))
    gpui = next(p for p in metadata["packages"] if p["name"] == "gpui")
    expected = ROOT / "app/gpui/vendor/gpui/Cargo.toml"
    if Path(gpui["manifest_path"]).resolve() != expected.resolve():
        raise ValueError("Cargo resolved another checkout's vendored GPUI")
    if not args.skip_build:
        helper_env = env.copy()
        helper_env.pop("CARGO_TARGET_DIR")
        subprocess.run([sys.executable, "scripts/build_capture_helpers.py", "--release"], cwd=ROOT, env=helper_env, check=True)
        subprocess.run(["cargo", "build", "--locked", "--release", "--features", "production-host", "--bin", "ui"], cwd=ROOT / "app/gpui", env=env, check=True)
    binary = target / "release/ui"
    native = bundle(binary, output / "native/Murmur.app")
    links = check_links(native / "Contents/MacOS/ui")
    # Separate payload directory: never replace any previously built Tauri app.
    oracle_target = output / "tauri-target"
    (oracle_target / "release").mkdir(parents=True)
    shutil.copy2(binary, oracle_target / "release/ui")
    oracle_env = {**env, "CARGO_TARGET_DIR": str(oracle_target), "CI": "true"}
    # Disable only updater artifact creation; no identity/plist/layout overrides.
    subprocess.run(["npx", "--no-install", "tauri", "bundle", "--no-sign", "--bundles", "app",
                    "--config", '{"bundle":{"createUpdaterArtifacts":false}}'],
                   cwd=ROOT / "app", env=oracle_env, check=True)
    oracle = oracle_target / "release/bundle/macos/Murmur.app"
    for app in (native, oracle):
        finalize(app, "-")
    evidence = compare_bundles(native, oracle)
    # Prove refusal on the actual assembled bundle, without leaving stray files.
    extra = native / "Contents/MacOS/remaining-qa"
    extra.write_bytes(b"synthetic disallowed executable")
    try:
        try:
            require_exact_macos_executables(native, native / "Contents/MacOS/ui",
                                           [native / "Contents/MacOS" / name for name in HELPERS])
        except SystemExit:
            evidence["extra_executable_rejected"] = True
        else:
            raise ValueError("executable allowlist accepted a QA binary")
    finally:
        extra.unlink()
    # Removing the extra file restores the original signed resource set.
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(native)], check=True)
    version = run([str(native / "Contents/MacOS/ui"), "--version"]).strip()
    if version != bundle_info()["CFBundleVersion"]:
        raise ValueError("native binary and bundle versions differ")
    evidence["launch"] = isolated_launch(native, output, env)
    assessment = subprocess.run(["spctl", "--assess", "--type", "execute", str(native)], capture_output=True, text=True)
    evidence.update({"source_sha": run(["git", "rev-parse", "HEAD"], cwd=ROOT).strip(),
                     "vendor_manifest": str(expected), "version": version, "links": links,
                     "signing": "ad-hoc only; not a Developer ID or notarization proof",
                     "spctl_exit": assessment.returncode,
                     "spctl": assessment.stderr.strip(),
                     "oracle": "Tauri bundler with the same native payload; no Tauri host build",
                     "publication": False})
    (output / "rehearsal.json").write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")
    print(json.dumps(evidence, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
