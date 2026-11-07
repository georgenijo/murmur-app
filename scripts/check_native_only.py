#!/usr/bin/env python3
"""Reject the retired UI runtime, including optional/build/dev/target edges."""
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
FORBIDDEN = {'tauri', 'tao', 'wry', 'webkit2gtk', 'webview2-com', 'web-view'}


def forbidden(name):
    return name in FORBIDDEN or name.startswith(('tauri-', 'tauri_'))


def manifest_violations(text):
    doc = tomllib.loads(text)
    failures = []
    def walk(value):
        if not isinstance(value, dict):
            return
        for key, child in value.items():
            if key in ('dependencies', 'build-dependencies', 'dev-dependencies'):
                for alias, spec in child.items():
                    name = spec.get('package', alias) if isinstance(spec, dict) else alias
                    if forbidden(name):
                        failures.append(name)
            walk(child)
    walk(doc)
    return failures


def main():
    paths = subprocess.check_output(['git', 'ls-files'], cwd=ROOT, text=True).splitlines()
    for path in paths:
        if not (ROOT / path).exists():
            continue
        if path.startswith(('app/src/', 'app/src-tauri/', 'app/public/', 'app/visual-tests/')):
            raise SystemExit('Retired frontend/runtime is tracked: ' + path)
        if path.endswith('Cargo.toml'):
            bad = manifest_violations((ROOT / path).read_text())
            if bad:
                raise SystemExit(f'{path}: forbidden runtime dependencies: {bad}')
    for package in ('murmur-core', 'murmur-services', 'murmur-capture-helper-protocol'):
        tree = subprocess.check_output([
            'cargo', 'tree', '--locked', '--manifest-path', str(ROOT / 'app/Cargo.toml'),
            '-p', package, '--target', 'all', '--all-features', '--edges', 'all',
            '--prefix', 'none', '--format', '{p}',
        ], text=True)
        names = {line.split()[0] for line in tree.splitlines() if line.strip()}
        if package not in names:
            raise SystemExit('Missing dependency root: ' + package)
        bad = {name for name in names if forbidden(name) or name in {'murmur-webkit', 'objc2-web-kit'}}
        if bad:
            raise SystemExit(f'{package}: forbidden service edges: {sorted(bad)}')
    print('Native-only manifests and all-target service graphs passed; importer is standalone')


if __name__ == '__main__':
    main()
