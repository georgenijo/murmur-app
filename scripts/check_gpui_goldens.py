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


def check_inventory(stdout, prefix):
    inventories = [json.loads(line.split('=', 1)[1]) for line in stdout.splitlines()
                   if line.startswith(prefix + '=')]
    if len(inventories) != 1:
        raise AssertionError('missing native process WebKit inventory')
    inventory = inventories[0]
    for field in ('pid', 'webkitImages', 'axAttributed', 'unattributed',
                  'startupWebkitImages', 'linkageViolations'):
        if type(inventory[field]) is not int or inventory[field] < 0:
            raise AssertionError(f'invalid WebKit inventory field: {field}')
    if (inventory['unattributed'] != 0 or inventory['startupWebkitImages'] != 0
            or inventory['linkageViolations'] != 0
            or inventory['webkitImages'] != inventory['axAttributed']):
        raise AssertionError(f'unattributed/app-linked WebKit: {inventory}')
    return inventory


def main():
    if platform.system() != 'Darwin':
        raise SystemExit('Metal goldens require macOS')
    subprocess.run(['cargo', 'build', '--locked', '-p', 'murmur-ui', '--features', 'gallery',
                    '--bins'], cwd=UI, check=True)
    target = Path(json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=UI))['target_directory'])
    output = target / 'goldens'
    output.mkdir(parents=True, exist_ok=True)
    # A deterministic launch-only run exits before NSApplication/AX exposure,
    # independent of active VoiceOver clients. Never change system settings.
    baseline = subprocess.check_output([str(target / 'debug/overlay-qa')],
        env={**os.environ, 'MURMUR_WEBKIT_BASELINE_ONLY': '1'}, cwd=UI, text=True, timeout=30)
    if check_inventory(baseline, 'MURMUR_OVERLAY_INVENTORY')['webkitImages'] != 0:
        raise AssertionError('nonzero launch-only baseline')
    print('PASS zero-image launch baseline and native linkage policy')
    policy_log = baseline.splitlines()
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
    overlay_cases = [(placement, state) for placement in ('notch', 'pretend-notch', 'pill')
                     for state in ('idle', 'starting', 'recording', 'processing', 'recovering',
                                   'cancelled', 'failure', 'delivery', 'expanded', 'suggestion', 'disabled', 'disabled-expanded')
                     if not (placement == 'pill' and state in ('idle', 'disabled', 'disabled-expanded'))]
    overlay_cases += [(placement, 'delivery-min-height')
                      for placement in ('notch', 'pretend-notch')]
    for placement, state in overlay_cases:
        case = f'overlay-{placement}-{state}'
        paths = [output / f'{case}-{i}.png' for i in (1, 2)]
        for path in paths:
            env = {k: v for k, v in os.environ.items() if k not in ('MURMUR_DARK', 'MURMUR_PILL', 'MURMUR_PRETEND_NOTCH')}
            env.update(MURMUR_GOLDEN=str(path), MURMUR_OVERLAY_STATE=state, MURMUR_DARK='1')
            if placement == 'pretend-notch':
                env['MURMUR_PRETEND_NOTCH'] = '1'
            if placement == 'pill':
                env['MURMUR_PILL'] = '1'
            stdout = subprocess.check_output([str(target / 'debug/overlay-qa')], env=env, cwd=UI, timeout=60, text=True)
            check_inventory(stdout, 'MURMUR_OVERLAY_INVENTORY')
            policy_log.extend(line for line in stdout.splitlines() if line.startswith('MURMUR_'))
        if paths[0].read_bytes() != paths[1].read_bytes():
            raise AssertionError(f'non-deterministic {case}')
        results[case] = hashlib.sha256(paths[0].read_bytes()).hexdigest()
        print(f'PASS {case}: identical offscreen Metal images')
    preview_baseline = subprocess.check_output([str(target / 'debug/preview-qa')],
        env={**os.environ, 'MURMUR_WEBKIT_BASELINE_ONLY': '1'}, cwd=UI, text=True, timeout=30)
    if check_inventory(preview_baseline, 'MURMUR_PREVIEW_INVENTORY')['webkitImages'] != 0:
        raise AssertionError('nonzero preview launch-only baseline')
    policy_log.extend(preview_baseline.splitlines())
    print('PASS zero-image preview launch baseline and native linkage policy')
    # Preview captures contain synthetic text only. Inventory this process,
    # not other applications; WebKit destruction has its own standalone test.
    for placement in ('notch', 'pill-light', 'pill-dark'):
        for state in ('short', 'long', 'unicode', 'unbroken', 'lift', 'contrast', 'stable'):
            case = f'preview-{placement}-{state}'
            paths = [output / f'{case}-{i}.png' for i in (1, 2)]
            for path in paths:
                env = {k: v for k, v in os.environ.items()
                       if k not in ('MURMUR_PILL', 'MURMUR_DARK', 'MURMUR_PREVIEW_STATE')}
                env.update(MURMUR_GOLDEN=str(path), MURMUR_PREVIEW_STATE=state)
                if placement.startswith('pill'):
                    env['MURMUR_PILL'] = '1'
                if placement != 'pill-light':
                    env['MURMUR_DARK'] = '1'
                stdout = subprocess.check_output([str(target / 'debug/preview-qa')], env=env,
                                                 cwd=UI, timeout=60, text=True)
                check_inventory(stdout, 'MURMUR_PREVIEW_INVENTORY')
                policy_log.extend(line for line in stdout.splitlines() if line.startswith('MURMUR_'))
            if paths[0].read_bytes() != paths[1].read_bytes():
                raise AssertionError(f'non-deterministic {case}')
            results[case] = hashlib.sha256(paths[0].read_bytes()).hexdigest()
            print(f'PASS {case}: identical offscreen Metal images')
        if results[f'preview-{placement}-short'] == results[f'preview-{placement}-lift']:
            raise AssertionError('preview lift did not move the painted card')
    if results['preview-pill-light-short'] == results['preview-pill-dark-short']:
        raise AssertionError('preview system appearance fixtures did not differ')
    diagnostics_baseline = subprocess.check_output([str(target / 'debug/diagnostics-qa')],
        env={**os.environ, 'MURMUR_WEBKIT_BASELINE_ONLY': '1'}, cwd=UI, text=True, timeout=30)
    if check_inventory(diagnostics_baseline, 'MURMUR_DIAGNOSTICS_INVENTORY')['webkitImages'] != 0:
        raise AssertionError('nonzero diagnostics launch-only baseline')
    policy_log.extend(diagnostics_baseline.splitlines())
    print('PASS zero-image diagnostics launch baseline and native linkage policy')
    # Diagnostics uses only the synthetic adapter, with no visible AppKit window.
    for mode in ('light', 'dark', 'contrast'):
        for state in ('events', 'events-expanded', 'empty', 'loading', 'runs', 'run-detail',
                      'performance', 'latency', 'reports', 'reports-blocked', 'dictation',
                      'private-detail', 'store-unavailable'):
            case = f'diagnostics-{mode}-{state}'
            paths = [output / f'{case}-{i}.png' for i in (1, 2)]
            for path in paths:
                env = {k: v for k, v in os.environ.items()
                       if k not in ('MURMUR_DARK', 'MURMUR_CONTRAST', 'MURMUR_DIAGNOSTICS_STATE')}
                env.update(MURMUR_GOLDEN=str(path), MURMUR_DIAGNOSTICS_STATE=state)
                if mode == 'dark':
                    env['MURMUR_DARK'] = '1'
                if mode == 'contrast':
                    env['MURMUR_CONTRAST'] = '1'
                stdout = subprocess.check_output([str(target / 'debug/diagnostics-qa')],
                                                 env=env, cwd=UI, timeout=60, text=True)
                check_inventory(stdout, 'MURMUR_DIAGNOSTICS_INVENTORY')
                policy_log.extend(line for line in stdout.splitlines() if line.startswith('MURMUR_'))
            if paths[0].read_bytes() != paths[1].read_bytes():
                raise AssertionError(f'non-deterministic {case}')
            results[case] = hashlib.sha256(paths[0].read_bytes()).hexdigest()
            print(f'PASS {case}: identical offscreen Metal images')
        if results[f'diagnostics-{mode}-events'] == results[f'diagnostics-{mode}-events-expanded']:
            raise AssertionError('diagnostics disclosure did not change its frame')
        if len({results[f'diagnostics-{mode}-{s}'] for s in
                ('events', 'runs', 'run-detail', 'performance', 'latency', 'reports', 'dictation', 'private-detail')}) != 8:
            raise AssertionError('diagnostics screens rendered identically')
    if len({results[f'diagnostics-{mode}-events'] for mode in ('light', 'dark', 'contrast')}) != 3:
        raise AssertionError('diagnostics appearance variants rendered identically')
    # History: native production view, complete synthetic 200-row snapshot.
    # No native interaction, user history, pasteboard or memory measurement.
    for mode in ('light', 'dark', 'contrast'):
        for state in ('list', 'compact', 'detail', 'detail-last-page', 'search', 'empty', 'no-matches',
                      'loading', 'retention-off', 'migration', 'error', 'confirm'):
            case = f'history-{mode}-{state}'
            paths = [output / f'{case}-{i}.png' for i in (1, 2)]
            for path in paths:
                env = {k: v for k, v in os.environ.items()
                       if k not in ('MURMUR_DARK', 'MURMUR_CONTRAST', 'MURMUR_HISTORY_STATE')}
                env.update(MURMUR_GOLDEN=str(path), MURMUR_HISTORY_STATE=state, TZ='UTC')
                if mode == 'dark':
                    env['MURMUR_DARK'] = '1'
                if mode == 'contrast':
                    env['MURMUR_CONTRAST'] = '1'
                stdout = subprocess.check_output([str(target / 'debug/history-qa')],
                                                 env=env, cwd=UI, timeout=60, text=True)
                inventory = [json.loads(line.split('=', 1)[1]) for line in stdout.splitlines()
                             if line.startswith('MURMUR_HISTORY_INVENTORY=')]
                if len(inventory) != 1 or inventory[0]['webkitImages'] != 0:
                    raise AssertionError('missing or nonempty History WebKit inventory')
            if paths[0].read_bytes() != paths[1].read_bytes():
                raise AssertionError(f'non-deterministic {case}')
            results[case] = hashlib.sha256(paths[0].read_bytes()).hexdigest()
            print(f'PASS {case}: identical offscreen Metal images')
        if len({results[f'history-{mode}-{state}'] for state in
                ('list', 'detail', 'search', 'empty', 'no-matches', 'loading', 'confirm')}) != 7:
            raise AssertionError('History states rendered identically')
    if len({results[f'history-{mode}-list'] for mode in ('light', 'dark', 'contrast')}) != 3:
        raise AssertionError('History appearance variants rendered identically')
    (output / 'webkit-policy.log').write_text('\n'.join(policy_log) + '\n')
    (output / 'receipt.json').write_text(json.dumps({
        'os': platform.platform(), 'gpui': '654efae441bc8eb117d0042bb28cd7cf3570bdc9',
        'comparison': 'same-host repeated render, strict byte equality',
        'sha256': results,
    }, indent=2) + '\n')


if __name__ == '__main__':
    main()
