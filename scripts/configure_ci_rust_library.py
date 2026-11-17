#!/usr/bin/env python3
"""Stamp the CI dependency-cache identity into a test checkout manifest.

The library crate type is already ["lib"] (desktop builds do not need cdylib or
staticlib), so this only validates that and adds the cache recipe metadata."""

import argparse
import hashlib
from pathlib import Path
import re


CACHE_TABLE = "\n[package.metadata.murmur-ci-cache]\n"


def recipe_digest(workflow: Path, helper: Path) -> str:
    return hashlib.sha256(workflow.read_bytes() + b"\0" + helper.read_bytes()).hexdigest()


def configure(manifest: Path, recipe: str) -> None:
    if not re.fullmatch(r"[0-9a-f]{64}", recipe):
        raise ValueError("expected a SHA-256 build-recipe digest")
    original = manifest.read_text()
    base = original
    has_cache_table = "[package.metadata.murmur-ci-cache]" in original
    if has_cache_table:
        if original.count(CACHE_TABLE) != 1:
            raise ValueError("unexpected existing CI cache metadata")
        base, cache_metadata = original.split(CACHE_TABLE)
        if not re.fullmatch(r'recipe = "[0-9a-f]{64}"\n', cache_metadata):
            raise ValueError("unexpected existing CI cache metadata")
    section = re.search(r"(?ms)^\[lib\][ \t]*\n.*?(?=^\[|\Z)", base)
    if not section:
        raise ValueError("missing [lib] section")
    library = section.group()
    declarations = re.findall(r"(?m)^[ \t]*crate-type[ \t]*=", library)
    if len(declarations) != 1:
        raise ValueError("expected one library crate-type declaration")
    if not re.search(r'(?m)^crate-type = \["lib"\][ \t]*$', library):
        raise ValueError("unexpected library crate types; inspect the CI configuration")
    if not base.endswith("\n"):
        base += "\n"
    # rust-cache hashes package metadata, but Cargo does not use it to select
    # dependencies or compiler flags. New recipes can restore old dependencies
    # and let main save the complete new graph under a fresh immutable key.
    configured = base + CACHE_TABLE + f'recipe = "{recipe}"\n'
    if configured != original:
        manifest.write_text(configured)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).resolve().parents[1] / "app/src-tauri/Cargo.toml")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    recipe = recipe_digest(root / ".github/workflows/ci.yml", Path(__file__))
    configure(args.manifest, recipe)
    print(f"CI test checkout configured; recipe {recipe[:12]}")


if __name__ == "__main__":
    main()
