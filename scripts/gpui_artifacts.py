#!/usr/bin/env python3
"""Transfer only GPUI payload binaries across isolated, non-publishing jobs.

Hashes come from build job outputs, never from a manifest in the downloaded
archive. Extraction treats app build outputs as data: no links, paths, scripts
or extra entries may overwrite the fresh signing checkout or signer tool.
"""
import argparse
import hashlib
from pathlib import Path
import re
import shutil
import tarfile

from finalize_macos_bundle import HELPERS

BUILD_FILES = ("ui", *(f"{name}-aarch64-apple-darwin" for name in HELPERS))
SIGNER_FILES = ("murmur-updater-signer",)


def files(kind: str) -> tuple[str, ...]:
    if kind not in ("build", "signer"):
        raise ValueError("unknown artifact kind")
    return BUILD_FILES if kind == "build" else SIGNER_FILES


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pack(kind: str, archive: Path, binary: Path, helpers_dir: Path | None = None) -> str:
    sources = {files(kind)[0]: binary}
    if kind == "build":
        if helpers_dir is None:
            raise ValueError("build archive requires helpers directory")
        sources.update({name: helpers_dir / name for name in BUILD_FILES[1:]})
    for source in sources.values():
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"expected regular binary: {source}")
    with tarfile.open(archive, "x") as output:
        for name, source in sources.items():
            output.add(source, arcname=name, recursive=False)
    return sha256(archive)


def verify(kind: str, archive: Path, expected_sha256: str, output: Path) -> None:
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha256) or sha256(archive) != expected_sha256:
        raise ValueError("artifact sha256 mismatch")
    with tarfile.open(archive, "r:") as source:
        members = source.getmembers()
        expected = files(kind)
        if (len(members) != len(expected) or {m.name for m in members} != set(expected)
                or any(not m.isfile() for m in members)):
            raise ValueError("artifact must contain exactly the allowlisted regular binaries")
        output.mkdir(parents=True, exist_ok=False)
        for member in members:
            # Never use extract/extractall on a dependency-controlled archive.
            with source.extractfile(member) as binary, (output / member.name).open("xb") as target:
                shutil.copyfileobj(binary, target)
            (output / member.name).chmod(0o755)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("pack", "verify"))
    parser.add_argument("kind", choices=("build", "signer"))
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--helpers-dir", type=Path)
    parser.add_argument("--sha256")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.operation == "pack":
        if args.binary is None:
            parser.error("pack requires --binary")
        print(pack(args.kind, args.archive, args.binary, args.helpers_dir))
    else:
        if args.sha256 is None or args.output is None:
            parser.error("verify requires --sha256 and --output")
        verify(args.kind, args.archive, args.sha256, args.output)


if __name__ == "__main__":
    main()
