#!/usr/bin/env python3
"""Trusted-main, manual-only signed GPUI rehearsal. Never promotes artifacts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

from bundle_gpui import ROOT, bundle, bundle_info
from finalize_macos_bundle import HELPERS, entitlements, signature_details
from rehearse_gpui_bundle import check_links, finalize, run
from release_artifacts import record_platform
from release_bundle_target import validate_target


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    validate_target(os.environ["GITHUB_EVENT_NAME"], "gpui")
    if os.environ["GITHUB_REF"] != "refs/heads/main":
        parser.error("signed rehearsals require trusted main")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    target = args.target_dir.resolve()
    env = {**os.environ, "CARGO_TARGET_DIR": str(target), "RUSTUP_TOOLCHAIN": "1.98.1",
           "MACOSX_DEPLOYMENT_TARGET": "14.0"}
    subprocess.run(["cargo", "build", "--locked", "--release", "--bin", "ui"], cwd=ROOT / "app/gpui", env=env, check=True)
    subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path", str(ROOT / "scripts/updater-signer/Cargo.toml")], env=env, check=True)
    app = bundle(target / "release/ui", output / "bundle/Murmur.app")
    check_links(app / "Contents/MacOS/ui")
    finalize(app, os.environ["APPLE_SIGNING_IDENTITY"], os.environ["APPLE_TEAM_ID"])
    subprocess.run([sys.executable, "scripts/notarize_and_repackage_macos.py", "--app", str(app),
                    "--app-dir", str(ROOT / "app"), "--identity", os.environ["APPLE_SIGNING_IDENTITY"],
                    "--version", bundle_info()["CFBundleVersion"], "--arch", "aarch64",
                    "--dmg-out", str(output / "dmg"),
                    "--native-signer", str(target / "release/murmur-updater-signer")], cwd=ROOT, check=True)
    subprocess.run(["xcrun", "stapler", "validate", str(app)], check=True)
    subprocess.run(["spctl", "--assess", "--type", "execute", "-vvv", str(app)], check=True)
    artifacts = output / "artifacts"
    artifacts.mkdir()
    for source in [*(output / "dmg").glob("*.dmg"), *app.parent.glob("*.app.tar.gz*")]:
        shutil.copy2(source, artifacts / source.name)
    helpers = {}
    for name in HELPERS:
        path = app / "Contents/MacOS" / name
        dr = re.search(r"^#*\s*designated => (.+)$", run(["codesign", "-d", "-r-", str(path)], stderr=subprocess.STDOUT), re.M)
        team = re.search(r"^TeamIdentifier=(.+)$", signature_details(path), re.M)
        if not dr or not team:
            raise ValueError("missing signed helper requirement or Team ID")
        import plistlib
        helpers[name.removeprefix("murmur-").replace("-", "_")] = {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "architecture": run(["lipo", "-archs", str(path)]).strip(),
            "designated_requirement": dr.group(1), "team_id": team.group(1),
            "entitlement_sha256": hashlib.sha256(plistlib.dumps(entitlements(path), sort_keys=True)).hexdigest(),
        }
    provenance = record_platform("macos", "darwin-aarch64", artifacts,
                                 os.environ["GITHUB_SHA"], os.environ["GITHUB_RUN_ID"], **helpers)
    provenance.update({"bundle_target": "gpui", "publication": False})
    (artifacts / "provenance.json").write_text(json.dumps(provenance, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
