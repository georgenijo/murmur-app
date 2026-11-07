#!/usr/bin/env python3
"""Exercise the real GPUI AX tree without AppKit windows or OS automation.

The pinned TestWindow drops AX callbacks. Materialize *only* its Apache-2.0
GPUI crate in the task target directory and add the test-only transport shim
recorded in app/gpui/patches/test-accessibility.patch. Engine production code
and the committed lockfile stay at the exact Phase 0 pin. No shared Cargo
checkout is modified. Python 3.11+, git, cargo; run on macOS with Rust 1.98.1.
"""
from pathlib import Path
import json
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
UI = ROOT / 'app/gpui'
REV = '654efae441bc8eb117d0042bb28cd7cf3570bdc9'


def inline(value):
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, dict):
        return '{ ' + ', '.join(json.dumps(k) + ' = ' + inline(v) for k, v in value.items()) + ' }'
    if isinstance(value, list):
        return '[' + ', '.join(inline(v) for v in value) + ']'
    return json.dumps(value)


def main():
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--format-version=1'], cwd=UI))
    package = next(p for p in metadata['packages'] if p['name'] == 'gpui')
    if not package['source'].endswith('#' + REV):
        raise RuntimeError('GPUI source differs from the Phase 0 pin')
    source = Path(package['manifest_path']).parent
    upstream = tomllib.loads((source.parent.parent / 'Cargo.toml').read_text())
    manifest = tomllib.loads((source / 'Cargo.toml').read_text())
    target = Path(metadata['target_directory']) / 'murmur-a11y-test-source'
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(source, target)
    # Resolve workspace inheritance without copying any other Zed crate.
    sections = {'package': manifest['package'], 'lib': manifest['lib'], 'features': manifest['features']}
    sections['package']['edition'] = upstream['workspace']['package']['edition']
    sections['package'].pop('edition.workspace', None)
    sections['package']['autotests'] = False
    sections['package']['autoexamples'] = False
    dependency_sections = [('dependencies', manifest['dependencies']), ('build-dependencies', manifest.get('build-dependencies', {}))]
    for cfg, values in manifest.get('target', {}).items():
        if 'dependencies' in values:
            dependency_sections.append(('target.' + json.dumps(cfg) + '.dependencies', values['dependencies']))
    for name, dependencies in dependency_sections:
        resolved = {}
        for key, value in dependencies.items():
            if isinstance(value, dict) and value.get('workspace'):
                extra = {k: v for k, v in value.items() if k != 'workspace'}
                value = upstream['workspace']['dependencies'][key]
                value = {'version': value} if isinstance(value, str) else value.copy()
                if 'path' in value:
                    value.pop('path')
                    value['git'] = 'https://github.com/zed-industries/zed'
                    value['rev'] = REV
                features = value.get('features', []) + extra.pop('features', [])
                value.update(extra)
                if features:
                    value['features'] = list(dict.fromkeys(features))
            resolved[key] = value
        sections[name] = resolved
    sections['workspace'] = {}
    (target / 'Cargo.toml').write_text('\n'.join(
        '[' + name + ']\n' + '\n'.join(json.dumps(k) + ' = ' + inline(v) for k, v in section.items()) + '\n'
        for name, section in sections.items()))
    subprocess.run(['git', 'apply', '--unsafe-paths', '--directory=' + str(target),
                    str(UI / 'patches/test-accessibility.patch')], check=True, cwd=target)
    lock = UI / 'Cargo.lock'
    original = lock.read_bytes()
    try:
        config = 'patch."https://github.com/zed-industries/zed".gpui.path=' + json.dumps(str(target))
        return subprocess.call(['cargo', '--config', config, 'test', '-p', 'murmur-ui',
                                '--features', 'a11y-tests', *sys.argv[1:]], cwd=UI)
    finally:
        lock.write_bytes(original)


if __name__ == '__main__':
    sys.exit(main())
