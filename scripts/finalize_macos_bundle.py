#!/usr/bin/env python3
"""Sign and fail-closed verify a Murmur app with dedicated helper entitlements."""

from __future__ import annotations

import argparse
from pathlib import Path
import plistlib
import subprocess


HELPER_NAME = "murmur-llm-sidecar"
HELPER_IDENTIFIER = "com.localdictation.local-llm-sidecar"


def run(command: list[str], *, capture: bool = False) -> str:
    result = subprocess.run(command, text=True, check=True, capture_output=capture)
    return result.stdout if capture else ""


def entitlements(path: Path) -> dict[str, object]:
    result = subprocess.run(
        ["codesign", "-d", "--entitlements", ":-", "--xml", str(path)],
        check=True,
        capture_output=True,
    )
    payload = result.stdout or result.stderr
    start = payload.find(b"<?xml")
    if start < 0:
        raise ValueError(f"codesign did not return entitlements for {path}")
    return plistlib.loads(payload[start:])


def sign(
    path: Path,
    identity: str,
    entitlement_file: Path | None,
    identifier: str | None = None,
) -> None:
    command = ["codesign", "--force", "--sign", identity, "--options", "runtime"]
    command.append("--timestamp=none" if identity == "-" else "--timestamp")
    if identifier is not None:
        command.extend(["--identifier", identifier])
    if entitlement_file is not None:
        command.extend(["--entitlements", str(entitlement_file)])
    command.append(str(path))
    run(command)


# Mach-O magic numbers (thin 32/64-bit both endiannesses, and fat/universal).
_MACHO_MAGICS = {
    b"\xcf\xfa\xed\xfe",  # 64-bit, little-endian (arm64/x86_64)
    b"\xce\xfa\xed\xfe",  # 32-bit, little-endian
    b"\xfe\xed\xfa\xcf",  # 64-bit, big-endian
    b"\xfe\xed\xfa\xce",  # 32-bit, big-endian
    b"\xca\xfe\xba\xbe",  # fat/universal, big-endian
    b"\xbe\xba\xfe\xca",  # fat/universal, little-endian
}


def _is_macho(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return handle.read(4) in _MACHO_MAGICS
    except OSError:
        return False


def sign_nested_code(app: Path, identity: str, exclude: set[Path]) -> None:
    """Sign every nested Mach-O in the bundle inside-out (deepest first).

    Notarization rejects any nested Mach-O that is ad-hoc/unsigned or lacks the
    hardened runtime or a secure timestamp, so this scans the whole bundle by
    Mach-O magic (not only ``Contents/Frameworks`` by extension) and signs each
    with Developer ID + hardened runtime + secure timestamp. The main executable
    and the helper are excluded here because they are signed immediately after
    with their own per-binary entitlements and identifiers.
    """
    contents = app / "Contents"
    if not contents.is_dir():
        return
    excluded = {path.resolve() for path in exclude}
    candidates = {
        path
        for path in contents.rglob("*")
        if path.is_file()
        and not path.is_symlink()
        and path.resolve() not in excluded
        and (
            path.suffix in {".dylib", ".so"}
            or path.parent.suffix == ".framework"
            or _is_macho(path)
        )
    }
    for path in sorted(candidates, key=lambda path: len(path.parts), reverse=True):
        sign(path, identity, None)


def require_exact(actual: dict[str, object], expected_path: Path, label: str) -> None:
    with expected_path.open("rb") as handle:
        expected = plistlib.load(handle)
    if actual != expected:
        raise SystemExit(f"{label} entitlements differ: expected={expected!r} actual={actual!r}")


def signature_details(path: Path) -> str:
    result = subprocess.run(
        ["codesign", "-d", "--verbose=4", str(path)],
        text=True,
        check=True,
        capture_output=True,
    )
    return result.stdout + result.stderr


def require_exact_macos_executables(
    app: Path, main_binary: Path, helper: Path
) -> None:
    """Fail closed unless the app ships exactly its two production executables."""
    executable_dir = app / "Contents" / "MacOS"
    expected = {main_binary.name, helper.name}
    actual = {path.name for path in executable_dir.iterdir()}
    if actual != expected:
        raise SystemExit(
            "app bundle executables differ: "
            f"expected={sorted(expected)!r} actual={sorted(actual)!r}"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--identity", required=True)
    parser.add_argument("--main-entitlements", type=Path, required=True)
    parser.add_argument("--helper-entitlements", type=Path, required=True)
    parser.add_argument("--expected-team-id")
    args = parser.parse_args()

    app = args.app.resolve()
    info_plist = app / "Contents" / "Info.plist"
    helper = app / "Contents" / "MacOS" / HELPER_NAME
    if not info_plist.is_file() or not helper.is_file():
        raise SystemExit("app bundle is missing Info.plist or the local-LLM helper")
    with info_plist.open("rb") as handle:
        main_name = plistlib.load(handle).get("CFBundleExecutable")
    main_binary = app / "Contents" / "MacOS" / str(main_name)
    if not main_binary.is_file() or main_binary == helper:
        raise SystemExit("app bundle has an invalid main executable")
    require_exact_macos_executables(app, main_binary, helper)

    sign_nested_code(app, args.identity, exclude={helper, main_binary})
    sign(helper, args.identity, args.helper_entitlements, HELPER_IDENTIFIER)
    sign(main_binary, args.identity, args.main_entitlements)
    sign(app, args.identity, args.main_entitlements)

    run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app)])
    require_exact(entitlements(helper), args.helper_entitlements, "helper")
    require_exact(entitlements(main_binary), args.main_entitlements, "main executable")

    helper_details = signature_details(helper)
    main_details = signature_details(main_binary)
    if "runtime" not in helper_details.lower() or "runtime" not in main_details.lower():
        raise SystemExit("helper and main executable must both use hardened runtime")
    if f"Identifier={HELPER_IDENTIFIER}" not in helper_details:
        raise SystemExit("helper code signature has the wrong fixed identifier")
    if args.expected_team_id:
        marker = f"TeamIdentifier={args.expected_team_id}"
        if marker not in helper_details or marker not in main_details:
            raise SystemExit("helper and main executable do not share the expected Team ID")

    helper_archs = run(["lipo", "-archs", str(helper)], capture=True).strip().split()
    if helper_archs != ["arm64"]:
        raise SystemExit(f"helper architecture must be exactly arm64, found {helper_archs}")
    print(f"finalized and verified {app}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
