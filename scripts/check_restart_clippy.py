#!/usr/bin/env python3
"""Compile synthetic bypass fixtures; never execute an exit or access OS input.
Run in a disposable Mac checkout. Clippy resolves real libc/Tauri definitions.
"""
from pathlib import Path
import subprocess
import sys
from check_restart_policy import violations

ROOT = Path(__file__).resolve().parents[1]

def main():
    # Foreign aliases have no libc definition for Clippy to resolve. The source
    # guard must reject these before compiling the existing resolved fixtures.
    for fixture in sorted((ROOT / 'scripts/fixtures/restart-foreign').glob('*.txt')):
        allowed = not violations(fixture.read_text())
        if allowed != fixture.stem.startswith('benign_'):
            print(f'FAIL foreign restart policy fixture: {fixture.stem}')
            return 1
        print(f'PASS foreign restart policy fixture: {fixture.stem}', flush=True)
    directory = ROOT / 'app/src-tauri/examples'
    for fixture in sorted((ROOT / 'scripts/fixtures/restart-clippy').glob('*.txt')):
        name = '__murmur_restart_guard_' + fixture.stem
        target = directory / (name + '.rs')
        created = False
        try:
            with target.open('x') as output:
                created = True
                output.write(fixture.read_text())
            # Newer toolchains add lints that pre-existing code trips; older ones
            # (CI) don't know them, so allow unknown lint names too.
            command = ['cargo', 'clippy', '--example', name, '--', '-D', 'warnings',
                       '-A', 'unknown-lints',
                       '-A', 'clippy::chunks_exact_to_as_chunks', '-A', 'clippy::byte_char_slices']
            result = subprocess.run(command, cwd=ROOT / 'app/src-tauri', text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            positive = fixture.stem == 'unrelated_exit'
            passed = result.returncode == 0 if positive else (
                result.returncode != 0 and 'clippy::disallowed_methods' in result.stdout
                and 'use of a disallowed method' in result.stdout)
            if not passed:
                print(result.stdout)
                print(f'FAIL resolved restart policy fixture: {fixture.stem}')
                return 1
            print(f'PASS resolved restart policy fixture: {fixture.stem}', flush=True)
        finally:
            if created:
                target.unlink(missing_ok=True)
    return 0

if __name__ == '__main__':
    sys.exit(main())
