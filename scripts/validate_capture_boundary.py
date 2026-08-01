#!/usr/bin/env python3
"""Fail CI if production HAL ownership leaks back into the Tauri app process."""

from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[1]
APP_MANIFEST = ROOT / "app/src-tauri/Cargo.toml"
HELPER_MANIFEST = ROOT / "app/src-tauri/sidecars/capture/Cargo.toml"
APP_SOURCE = ROOT / "app/src-tauri/src"


def dependency_names(table: dict) -> set[str]:
    names: set[str] = set(table.get("dependencies", {}))
    for value in table.get("target", {}).values():
        names.update(value.get("dependencies", {}))
    return names


def main() -> None:
    app = tomllib.loads(APP_MANIFEST.read_text())
    helper = tomllib.loads(HELPER_MANIFEST.read_text())
    forbidden = {"cpal", "coreaudio", "coreaudio-rs"}
    leaked = dependency_names(app) & forbidden
    if leaked:
        raise SystemExit(f"app process links forbidden HAL crates: {sorted(leaked)}")
    missing = {"cpal", "coreaudio-rs"} - dependency_names(helper)
    if missing:
        raise SystemExit(f"capture worker is missing backend crates: {sorted(missing)}")
    pattern = re.compile(
        r"\b(?:cpal|coreaudio)::|AudioUnit(?:Initialize|Start|Stop)|"
        r"kAudioOutputUnitProperty_CurrentDevice"
    )
    violations: list[str] = []
    for path in APP_SOURCE.rglob("*.rs"):
        if path.name in {"capture_helper_probe.rs", "capture_agent_probe.rs"}:
            continue
        for number, line in enumerate(path.read_text(errors="replace").splitlines(), 1):
            if pattern.search(line):
                violations.append(f"{path.relative_to(ROOT)}:{number}")
    if violations:
        raise SystemExit("app-process HAL source found:\n" + "\n".join(violations))
    print("capture boundary valid: production HAL crates are worker-only")


if __name__ == "__main__":
    main()
