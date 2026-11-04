#!/usr/bin/env python3
"""Use only Rust library outputs in an isolated CI test checkout."""

import argparse
from pathlib import Path
import re


def configure(manifest: Path) -> None:
    original = manifest.read_text()
    section = re.search(r"(?ms)^\[lib\][ \t]*\n.*?(?=^\[|\Z)", original)
    if not section:
        raise ValueError("missing [lib] section")
    library = section.group()
    declarations = re.findall(r"(?m)^[ \t]*crate-type[ \t]*=", library)
    if len(declarations) != 1:
        raise ValueError("expected one library crate-type declaration")
    canonical = r'(?m)^crate-type = \["lib", "cdylib", "staticlib"\][ \t]*$'
    test_only = 'crate-type = ["lib"]'
    if re.search(r'(?m)^crate-type = \["lib"\][ \t]*$', library):
        return
    if not re.search(canonical, library):
        raise ValueError("unexpected library crate types; inspect the CI configuration")
    changed = re.sub(canonical, test_only, library, count=1)
    manifest.write_text(original[:section.start()] + changed + original[section.end():])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).resolve().parents[1] / "app/src-tauri/Cargo.toml")
    args = parser.parse_args()
    configure(args.manifest)
    print("CI test checkout configured for Rust library output")


if __name__ == "__main__":
    main()
