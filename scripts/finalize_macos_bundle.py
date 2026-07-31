#!/usr/bin/env python3
"""Sign and fail-closed verify a Murmur app with dedicated helper entitlements."""

from __future__ import annotations

import argparse
from pathlib import Path
import plistlib
import shutil
import subprocess


HELPERS = {
    "murmur-capture-agent": "com.localdictation.capture-agent",
    "murmur-capture-helper": "com.localdictation.capture-helper",
    "murmur-capture-worker": "com.localdictation.capture-worker",
    "murmur-llm-sidecar": "com.localdictation.local-llm-sidecar",
}
CAPTURE_AGENT_PLIST = "com.localdictation.capture-agent.plist"
CAPTURE_AGENT_MACH_SERVICE = "com.localdictation.capture-agent.xpc"


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
    app: Path, main_binary: Path, helpers: list[Path]
) -> None:
    """Fail closed unless the app ships exactly its production executables."""
    executable_dir = app / "Contents" / "MacOS"
    expected = {main_binary.name, *(helper.name for helper in helpers)}
    actual = {path.name for path in executable_dir.iterdir()}
    if actual != expected:
        raise SystemExit(
            "app bundle executables differ: "
            f"expected={sorted(expected)!r} actual={sorted(actual)!r}"
        )


def install_capture_agent_plist(app: Path, source: Path) -> Path:
    with source.open("rb") as handle:
        payload = plistlib.load(handle)
    expected = {
        "Label": "com.localdictation.capture-agent",
        "BundleProgram": "Contents/MacOS/murmur-capture-agent",
        "MachServices": {CAPTURE_AGENT_MACH_SERVICE: True},
        "ProcessType": "Interactive",
        "ThrottleInterval": 10,
    }
    if payload != expected:
        raise SystemExit(
            f"capture-agent launchd plist differs: expected={expected!r} actual={payload!r}"
        )
    destination = app / "Contents" / "Library" / "LaunchAgents" / CAPTURE_AGENT_PLIST
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    return destination


def decode_otool_info_plist(output: str) -> dict[str, object]:
    payload = bytearray()
    for line in output.splitlines():
        fields = line.split()
        if len(fields) < 2:
            continue
        try:
            int(fields[0], 16)
        except ValueError:
            continue
        for word in fields[1:]:
            if len(word) != 8:
                raise SystemExit("embedded Info.plist has malformed words")
            try:
                payload.extend(bytes.fromhex(word)[::-1])
            except ValueError as exc:
                raise SystemExit(
                    "embedded Info.plist is not hexadecimal"
                ) from exc
    try:
        value = plistlib.loads(bytes(payload).rstrip(b"\0"))
    except Exception as exc:
        raise SystemExit("embedded Info.plist is invalid") from exc
    if not isinstance(value, dict):
        raise SystemExit("embedded Info.plist is not a dictionary")
    return value


def require_embedded_info(
    executable: Path,
    template: Path,
    app_version: str,
    label: str,
) -> None:
    with template.open("rb") as handle:
        expected = plistlib.load(handle)
    for key in ("CFBundleShortVersionString", "CFBundleVersion"):
        expected[key] = app_version
    actual = decode_otool_info_plist(
        run(
            ["otool", "-X", "-s", "__TEXT", "__info_plist", str(executable)],
            capture=True,
        )
    )
    if actual != expected:
        raise SystemExit(
            f"{label} embedded Info.plist differs: "
            f"expected={expected!r} actual={actual!r}"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--identity", required=True)
    parser.add_argument("--main-entitlements", type=Path, required=True)
    parser.add_argument(
        "--llm-helper-entitlements",
        "--helper-entitlements",
        dest="llm_helper_entitlements",
        type=Path,
        required=True,
    )
    parser.add_argument("--capture-helper-entitlements", type=Path, required=True)
    parser.add_argument("--capture-worker-entitlements", type=Path, required=True)
    parser.add_argument("--capture-agent-entitlements", type=Path, required=True)
    parser.add_argument("--capture-agent-info-plist", type=Path, required=True)
    parser.add_argument("--capture-helper-info-plist", type=Path, required=True)
    parser.add_argument("--capture-worker-info-plist", type=Path, required=True)
    parser.add_argument("--capture-agent-launchd-plist", type=Path, required=True)
    parser.add_argument("--expected-team-id")
    args = parser.parse_args()

    app = args.app.resolve()
    info_plist = app / "Contents" / "Info.plist"
    helpers = {
        name: app / "Contents" / "MacOS" / name for name in HELPERS
    }
    if not info_plist.is_file() or not all(path.is_file() for path in helpers.values()):
        raise SystemExit("app bundle is missing Info.plist or a required helper")
    with info_plist.open("rb") as handle:
        app_info = plistlib.load(handle)
    main_name = app_info.get("CFBundleExecutable")
    app_version = str(app_info.get("CFBundleVersion", ""))
    if not app_version:
        raise SystemExit("app bundle version is missing")
    main_binary = app / "Contents" / "MacOS" / str(main_name)
    if not main_binary.is_file() or main_binary in helpers.values():
        raise SystemExit("app bundle has an invalid main executable")
    require_exact_macos_executables(app, main_binary, list(helpers.values()))
    require_embedded_info(
        helpers["murmur-capture-agent"],
        args.capture_agent_info_plist,
        app_version,
        "capture agent",
    )
    require_embedded_info(
        helpers["murmur-capture-helper"],
        args.capture_helper_info_plist,
        app_version,
        "capture helper",
    )
    require_embedded_info(
        helpers["murmur-capture-worker"],
        args.capture_worker_info_plist,
        app_version,
        "capture worker",
    )
    launchd_plist = install_capture_agent_plist(app, args.capture_agent_launchd_plist)

    sign_nested_code(app, args.identity, exclude={main_binary, *helpers.values()})
    sign(
        helpers["murmur-capture-agent"],
        args.identity,
        args.capture_agent_entitlements,
        HELPERS["murmur-capture-agent"],
    )
    sign(
        helpers["murmur-capture-helper"],
        args.identity,
        args.capture_helper_entitlements,
        HELPERS["murmur-capture-helper"],
    )
    sign(
        helpers["murmur-capture-worker"],
        args.identity,
        args.capture_worker_entitlements,
        HELPERS["murmur-capture-worker"],
    )
    sign(
        helpers["murmur-llm-sidecar"],
        args.identity,
        args.llm_helper_entitlements,
        HELPERS["murmur-llm-sidecar"],
    )
    sign(main_binary, args.identity, args.main_entitlements)
    sign(app, args.identity, args.main_entitlements)

    run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app)])
    require_exact(
        entitlements(helpers["murmur-capture-agent"]),
        args.capture_agent_entitlements,
        "capture agent",
    )
    require_exact(
        entitlements(helpers["murmur-capture-helper"]),
        args.capture_helper_entitlements,
        "capture helper",
    )
    require_exact(
        entitlements(helpers["murmur-capture-worker"]),
        args.capture_worker_entitlements,
        "capture worker",
    )
    require_exact(
        entitlements(helpers["murmur-llm-sidecar"]),
        args.llm_helper_entitlements,
        "local-LLM helper",
    )
    require_exact(entitlements(main_binary), args.main_entitlements, "main executable")
    with launchd_plist.open("rb") as handle:
        if plistlib.load(handle)["MachServices"] != {CAPTURE_AGENT_MACH_SERVICE: True}:
            raise SystemExit("signed bundle capture-agent Mach service differs")

    helper_details = {
        name: signature_details(path) for name, path in helpers.items()
    }
    main_details = signature_details(main_binary)
    if (
        any("runtime" not in details.lower() for details in helper_details.values())
        or "runtime" not in main_details.lower()
    ):
        raise SystemExit("helpers and main executable must use hardened runtime")
    for name, identifier in HELPERS.items():
        if f"Identifier={identifier}" not in helper_details[name]:
            raise SystemExit(f"{name} code signature has the wrong fixed identifier")
    if args.expected_team_id:
        marker = f"TeamIdentifier={args.expected_team_id}"
        if (
            any(marker not in details for details in helper_details.values())
            or marker not in main_details
        ):
            raise SystemExit("helpers and main executable do not share the expected Team ID")

    for name, helper in helpers.items():
        helper_archs = run(["lipo", "-archs", str(helper)], capture=True).strip().split()
        if helper_archs != ["arm64"]:
            raise SystemExit(
                f"{name} architecture must be exactly arm64, found {helper_archs}"
            )
    print(f"finalized and verified {app}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
