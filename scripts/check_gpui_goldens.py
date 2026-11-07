#!/usr/bin/env python3
"""Render the synthetic gallery twice per mode to offscreen Metal textures.

The repeat comparison is strict byte equality, on one machine/OS/scale. Images
are review artifacts; cross-OS font rasterization is not a portable baseline.
No NSApplication, visible window, AX client, or system preference write.
"""
from pathlib import Path
import hashlib
import json
import os
import platform
import subprocess

ROOT = Path(__file__).resolve().parents[1]
UI = ROOT / 'app/gpui'


def main():
    if platform.system() != 'Darwin':
        raise SystemExit('Metal goldens require macOS')
    subprocess.run(['cargo', 'build', '--locked', '-p', 'murmur-ui', '--features', 'gallery',
                    '--bins'], cwd=UI, check=True)
    target = Path(json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=UI))['target_directory'])
    output = target / 'goldens'
    output.mkdir(parents=True, exist_ok=True)
    results = {}
    cases = [(mode, state) for mode in ('light', 'dark', 'contrast')
             for state in ('rest', 'hover', 'pressed', 'button', 'toggle', 'field', 'dropdown', 'popup', 'list', 'navigation')]
    for mode, state in cases:
        case = f'{mode}-{state}'
        paths = [output / f'{case}-{i}.png' for i in (1, 2)]
        for path in paths:
            env = {k: v for k, v in os.environ.items() if k not in ('MURMUR_DARK', 'MURMUR_CONTRAST')}
            env['MURMUR_GOLDEN'] = str(path)
            env['MURMUR_GALLERY_STATE'] = state
            if mode == 'dark':
                env['MURMUR_DARK'] = '1'
            if mode == 'contrast':
                env['MURMUR_CONTRAST'] = '1'
            subprocess.run([str(target / 'debug/control-gallery')], env=env, cwd=UI, timeout=60, check=True)
        if paths[0].read_bytes() != paths[1].read_bytes():
            raise AssertionError(f'non-deterministic {mode} golden; compare {paths}')
        results[case] = hashlib.sha256(paths[0].read_bytes()).hexdigest()
        print(f'PASS {case}: identical offscreen Metal images')
    for mode in ('light', 'dark', 'contrast'):
        if len({results[f'{mode}-{state}'] for state in ('rest', 'hover', 'pressed', 'button')}) != 4:
            raise AssertionError(f'{mode} resting/hover/pressed/focus states did not differ')
    if len({results[f'{mode}-rest'] for mode in ('light', 'dark', 'contrast')}) != 3:
        raise AssertionError('appearance/preference variants rendered identically')
    # Overlay uses the production view/controls, synthetic snapshots and the
    # same offscreen Metal platform. Never opens a real native window.
    overlay_cases = [(placement, state) for placement in ('notch', 'pill')
                     for state in ('idle', 'starting', 'recording', 'processing', 'recovering',
                                   'cancelled', 'failure', 'delivery', 'expanded', 'suggestion')
                     if (placement, state) != ('pill', 'idle')]
    for placement, state in overlay_cases:
        case = f'overlay-{placement}-{state}'
        paths = [output / f'{case}-{i}.png' for i in (1, 2)]
        for path in paths:
            env = {k: v for k, v in os.environ.items() if k not in ('MURMUR_DARK', 'MURMUR_PILL')}
            env.update(MURMUR_GOLDEN=str(path), MURMUR_OVERLAY_STATE=state, MURMUR_DARK='1')
            if placement == 'pill':
                env['MURMUR_PILL'] = '1'
            stdout = subprocess.check_output([str(target / 'debug/overlay-qa')], env=env, cwd=UI, timeout=60, text=True)
            inventory = [json.loads(line.split('=', 1)[1]) for line in stdout.splitlines()
                         if line.startswith('MURMUR_OVERLAY_INVENTORY=')]
            if len(inventory) != 1 or inventory[0]['webkitImages'] != 0:
                raise AssertionError('missing or nonempty native overlay process WebKit inventory')
        if paths[0].read_bytes() != paths[1].read_bytes():
            raise AssertionError(f'non-deterministic {case}')
        results[case] = hashlib.sha256(paths[0].read_bytes()).hexdigest()
        print(f'PASS {case}: identical offscreen Metal images')
    (output / 'receipt.json').write_text(json.dumps({
        'os': platform.platform(), 'gpui': '654efae441bc8eb117d0042bb28cd7cf3570bdc9',
        'comparison': 'same-host repeated render, strict byte equality',
        'sha256': results,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
