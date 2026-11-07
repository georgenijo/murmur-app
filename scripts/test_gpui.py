#!/usr/bin/env python3
"""Exercise the real GPUI AX tree without AppKit windows or OS automation.

The pinned TestWindow drops AX callbacks. Materialize *only* its Apache-2.0
vendored GPUI crate in the task target directory and add the test-only transport
shim recorded in app/gpui/patches/test-accessibility.patch. The production
deferred-AX fix is already present; only test transport is added here. The
committed lockfile is restored. No shared Cargo
checkout is modified. Python 3.11+, git, cargo; run on macOS with Rust 1.98.1.
"""
from pathlib import Path
import json
import os
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
UI = ROOT / 'app/gpui'
REV = '654efae441bc8eb117d0042bb28cd7cf3570bdc9'


def validated_source(package):
    source = Path(package['manifest_path']).parent
    expected = UI / 'vendor/gpui'
    provenance = tomllib.loads((source / 'Cargo.toml').read_text())
    revision = provenance.get('package', {}).get('metadata', {}).get('murmur', {}).get('upstream-revision')
    if source.resolve() != expected.resolve() or revision != REV:
        raise RuntimeError('GPUI source differs from the patched Phase 0 pin')
    return source


def main():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version=1'], cwd=UI))
    package = next(p for p in metadata['packages'] if p['name'] == 'gpui')
    source = validated_source(package)
    target = Path(metadata['target_directory']) / 'murmur-a11y-test-source'
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)
    # A target directory inside a work tree (CI's app/gpui/target) makes git
    # apply silently skip every path; the ceiling keeps it out of that repo.
    patch = str(UI / 'patches/test-accessibility.patch')
    env = {**os.environ, 'GIT_CEILING_DIRECTORIES': str(target.parent)}
    subprocess.run(['git', 'apply', patch], check=True, cwd=target, env=env)
    subprocess.run(['git', 'apply', '--check', '--reverse', patch], check=True, cwd=target, env=env)
    lock = UI.parent / 'Cargo.lock'
    original = lock.read_bytes()
    try:
        config = 'patch."https://github.com/zed-industries/zed".gpui.path=' + json.dumps(str(target))
        subprocess.run(['cargo', '--config', config, 'test', '-p', 'accesskit_macos', '--lib'],
                       cwd=UI, check=True)
        return subprocess.call(['cargo', '--config', config, 'test', '-p', 'murmur-ui',
                                '--features', 'a11y-tests', *sys.argv[1:]], cwd=UI)
    finally:
        lock.write_bytes(original)


if __name__ == '__main__':
    sys.exit(main())
