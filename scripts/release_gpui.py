#!/usr/bin/env python3
"""Trusted-main, manual-only signed GPUI rehearsal. Never promotes artifacts."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys

from bundle_gpui import ROOT, bundle, bundle_info
from finalize_macos_bundle import HELPERS, entitlements, signature_details
from rehearse_gpui_bundle import check_links, finalize, run
from release_artifacts import create_provenance
from release_bundle_target import validate_target


CREDENTIAL_PREFIXES = ("APPLE_", "TAURI_SIGNING_")
CREDENTIAL_MARKERS = ("PASSWORD", "SECRET", "TOKEN", "PRIVATE_KEY", "KEYCHAIN")


def is_credential(name: str) -> bool:
    upper = name.upper()
    return upper.startswith(CREDENTIAL_PREFIXES) or any(marker in upper for marker in CREDENTIAL_MARKERS)


def build(target: Path, *, signer_only: bool = False) -> None:
    """Compile with no credentials, so dependency build code never sees them."""
    present = sorted(name for name in os.environ if is_credential(name))
    if present:
        raise ValueError(f"build phase must run without credentials: {', '.join(present)}")
    env = {name: value for name, value in os.environ.items() if not is_credential(name)}
    env.update({"CARGO_TARGET_DIR": str(target), "RUSTUP_TOOLCHAIN": "1.98.1",
                "MACOSX_DEPLOYMENT_TARGET": "14.0"})
    if signer_only:
        subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path", str(ROOT / "scripts/updater-signer/Cargo.toml")], env=env, check=True)
    else:
        subprocess.run(["cargo", "build", "--locked", "--release", "--features", "production-host", "--bin", "ui"], cwd=ROOT / "app/gpui", env=env, check=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("build", "build-signer", "sign"))
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--helpers-dir", type=Path)
    parser.add_argument("--native-signer", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    validate_target(os.environ["GITHUB_EVENT_NAME"], "gpui")
    if os.environ["GITHUB_REF"] != "refs/heads/main":
        parser.error("signed rehearsals require trusted main")
    if args.phase in ("build", "build-signer"):
        if args.target_dir is None:
            parser.error("build requires --target-dir")
        build(args.target_dir.resolve(), signer_only=args.phase == "build-signer")
        return
    if any(value is None for value in (args.binary, args.helpers_dir, args.native_signer, args.output)):
        parser.error("sign requires --binary, --helpers-dir, --native-signer and --output")
    if args.target_dir is not None:
        parser.error("sign does not accept a build target directory")
    sign(args.binary.resolve(), args.helpers_dir.resolve(), args.native_signer.resolve(), args.output.resolve())


def sign(binary: Path, helpers_dir: Path, signer: Path, output: Path) -> None:
    """Use verified prebuilt inputs from isolated app and signer jobs. Never compile."""
    output.mkdir(parents=True, exist_ok=False)
    app = bundle(binary, output / "bundle/Murmur.app", helpers_dir=helpers_dir)
    check_links(app / "Contents/MacOS/ui")
    finalize(app, os.environ["APPLE_SIGNING_IDENTITY"], os.environ["APPLE_TEAM_ID"])
    subprocess.run([sys.executable, "scripts/notarize_and_repackage_macos.py", "--app", str(app),
                    "--app-dir", str(ROOT / "app"), "--identity", os.environ["APPLE_SIGNING_IDENTITY"],
                    "--version", bundle_info()["CFBundleVersion"], "--arch", "aarch64",
                    "--dmg-out", str(output / "dmg"),
                    "--native-signer", str(signer)], cwd=ROOT, check=True)
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
        helpers[name.removeprefix("murmur-").replace("-", "_")] = {
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            "architecture": run(["lipo", "-archs", str(path)]).strip(),
            "designated_requirement": dr.group(1), "team_id": team.group(1),
            "entitlement_sha256": hashlib.sha256(plistlib.dumps(entitlements(path), sort_keys=True)).hexdigest(),
        }
    provenance = create_provenance("macos", "darwin-aarch64", artifacts,
                                 os.environ["GITHUB_SHA"], os.environ["GITHUB_RUN_ID"], **helpers)
    provenance.update({"bundle_target": "gpui", "publication": False})
    (artifacts / "provenance.json").write_text(json.dumps(provenance, indent=2, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
